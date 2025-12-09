pub mod strategies;

use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;
use tracing::info;

pub use strategies::{StrategiesConfig, UpOrDownConfig};

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Failed to load config file: {0}")]
    FileError(#[from] std::io::Error),

    #[error("Failed to parse YAML: {0}")]
    YamlError(#[from] serde_yaml::Error),

    #[error("Environment variable not found: {0}")]
    EnvVarMissing(String),

    #[error("Invalid configuration: {0}")]
    ValidationError(String),
}

pub type Result<T> = std::result::Result<T, ConfigError>;

/// Main bot configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotConfig {
    pub database: DatabaseConfig,
    pub gamma_api: GammaApiConfig,
    pub trading: TradingConfig,
    pub risk: RiskConfig,
    pub polymarket: PolymarketConfig,
    pub scanner: ScannerConfig,

    /// Private key from .env (not in YAML)
    #[serde(skip)]
    pub private_key: String,

    /// Wallet address from .env (not in YAML)
    #[serde(skip)]
    pub wallet_address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    pub url: String,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self { url: String::new() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GammaApiConfig {
    pub base_url: String,
    pub sync_interval_secs: u64,
    pub initial_sync_on_startup: bool,
    #[serde(default)]
    pub filters: GammaFilters,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GammaFilters {
    pub min_liquidity: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradingConfig {
    pub probability_threshold: f64,
    pub seconds_before_resolution: u64,
    pub bet_amount_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskConfig {
    pub max_concurrent_positions: usize,
    pub max_bet_per_market: f64,
    pub daily_loss_limit: f64,
    pub min_profit_cents: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolymarketConfig {
    pub clob_url: String,
    pub ws_url: String,
    pub chain_id: u64,
    pub signature_type: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScannerConfig {
    pub poll_interval_secs: u64,
    pub min_resolution_window_mins: u64,
}

/// Market Sniper configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SniperConfig {
    pub probability: f64,
    pub delta_t_seconds: f64,
    pub loop_interval_secs: f64,
    /// Log level (error, warn, info, debug, trace)
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default)]
    pub database: DatabaseConfig,
}

impl SniperConfig {
    /// Load configuration from YAML file
    pub fn load(config_path: impl AsRef<Path>) -> Result<Self> {
        // Load YAML config
        let yaml_content = std::fs::read_to_string(config_path)?;
        let mut config: SniperConfig = serde_yaml::from_str(&yaml_content)?;

        // Override database URL from environment if present
        if let Ok(db_url) = std::env::var("DATABASE_URL") {
            info!("Overriding database URL from environment variable");
            config.database.url = db_url;
        }

        // Validate configuration
        config.validate()?;

        Ok(config)
    }

    /// Validate configuration values
    fn validate(&self) -> Result<()> {
        // Validate probability
        if self.probability < 0.0 || self.probability > 1.0 {
            return Err(ConfigError::ValidationError(
                "probability must be between 0 and 1".to_string(),
            ));
        }

        // Validate delta_t_seconds
        if self.delta_t_seconds <= 0.0 {
            return Err(ConfigError::ValidationError(
                "delta_t_seconds must be greater than 0".to_string(),
            ));
        }

        // Validate loop_interval_secs
        if self.loop_interval_secs <= 0.0 {
            return Err(ConfigError::ValidationError(
                "loop_interval_secs must be greater than 0".to_string(),
            ));
        }

        // Validate log_level
        let valid_levels = ["error", "warn", "info", "debug", "trace"];
        if !valid_levels.contains(&self.log_level.to_lowercase().as_str()) {
            return Err(ConfigError::ValidationError(
                format!("log_level must be one of: {}", valid_levels.join(", ")),
            ));
        }

        Ok(())
    }

    /// Log configuration summary
    pub fn log(&self) {
        info!("Configuration loaded:");
        info!("  Probability threshold: {}", self.probability);
        info!("  Time window: {} seconds", self.delta_t_seconds);
        info!("  Loop interval: {} seconds", self.loop_interval_secs);
        info!("  Log level: {}", self.log_level);
        info!("  Database url: {}", self.database.url);
    }
}

/// Events Syncer configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventsConfig {
    /// Gamma API base URL
    pub gamma_api_url: String,
    /// Sync interval in seconds
    #[serde(default = "default_sync_interval")]
    pub sync_interval_secs: u64,
    /// Log level (error, warn, info, debug, trace)
    #[serde(default = "default_log_level")]
    pub log_level: String,
    /// Database configuration (loaded from env)
    #[serde(default)]
    pub database: DatabaseConfig,
    /// Whether to fetch closed events
    /// false = add closed=false param (fetch only non-closed)
    /// true = omit closed param (fetch all including closed)
    #[serde(default)]
    pub closed: bool,
}

fn default_sync_interval() -> u64 {
    60
}

fn default_log_level() -> String {
    "info".to_string()
}

impl EventsConfig {
    /// Load configuration from YAML file
    pub fn load(config_path: impl AsRef<Path>) -> Result<Self> {
        let yaml_content = std::fs::read_to_string(config_path)?;
        let mut config: EventsConfig = serde_yaml::from_str(&yaml_content)?;

        // Override database URL from environment if present
        if let Ok(db_url) = std::env::var("DATABASE_URL") {
            info!("Overriding database URL from environment variable");
            config.database.url = db_url;
        }

        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        if self.gamma_api_url.is_empty() {
            return Err(ConfigError::ValidationError(
                "gamma_api_url cannot be empty".to_string(),
            ));
        }
        if self.sync_interval_secs == 0 {
            return Err(ConfigError::ValidationError(
                "sync_interval_secs must be greater than 0".to_string(),
            ));
        }
        let valid_levels = ["error", "warn", "info", "debug", "trace"];
        if !valid_levels.contains(&self.log_level.to_lowercase().as_str()) {
            return Err(ConfigError::ValidationError(
                format!("log_level must be one of: {}", valid_levels.join(", ")),
            ));
        }
        Ok(())
    }

    /// Log configuration summary
    pub fn log(&self) {
        info!("Configuration loaded:");
        info!("  Gamma API URL: {}", self.gamma_api_url);
        info!("  Sync interval: {} seconds", self.sync_interval_secs);
        info!("  Log level: {}", self.log_level);
        info!("  Closed filter: {}", if self.closed { "fetch all (no filter)" } else { "fetch non-closed only" });
        info!("  Database URL: {}", self.database.url);
    }
}

impl BotConfig {
    /// Load configuration from YAML file and .env
    pub fn load(config_path: impl AsRef<Path>) -> Result<Self> {
        // Load YAML config
        let yaml_content = std::fs::read_to_string(config_path)?;
        let mut config: BotConfig = serde_yaml::from_str(&yaml_content)?;

        // Load .env file
        dotenv::dotenv().ok(); // Don't fail if .env doesn't exist

        // Load private key from environment
        config.private_key = std::env::var("PRIVATE_KEY")
            .map_err(|_| ConfigError::EnvVarMissing("PRIVATE_KEY".to_string()))?;

        // Load wallet address from environment
        config.wallet_address = std::env::var("WALLET_ADDRESS")
            .map_err(|_| ConfigError::EnvVarMissing("WALLET_ADDRESS".to_string()))?;

        // Override database URL from environment if present
        if let Ok(db_url) = std::env::var("DATABASE_URL") {
            info!("Overriding database URL from environment variable");
            config.database.url = db_url;
        }

        // Validate configuration
        config.validate()?;

        Ok(config)
    }

    /// Validate configuration values
    fn validate(&self) -> Result<()> {
        // Validate probability threshold
        if self.trading.probability_threshold < 0.0 || self.trading.probability_threshold > 1.0 {
            return Err(ConfigError::ValidationError(
                "probability_threshold must be between 0 and 1".to_string(),
            ));
        }

        // Validate bet amounts
        if self.trading.bet_amount_usd <= 0.0 {
            return Err(ConfigError::ValidationError(
                "bet_amount_usd must be positive".to_string(),
            ));
        }

        if self.risk.max_bet_per_market < self.trading.bet_amount_usd {
            return Err(ConfigError::ValidationError(
                "max_bet_per_market must be >= bet_amount_usd".to_string(),
            ));
        }

        // Validate private key format (should start with 0x and be 64 hex chars + 0x)
        if !self.private_key.starts_with("0x") || self.private_key.len() != 66 {
            return Err(ConfigError::ValidationError(
                "PRIVATE_KEY must be a valid hex string (0x followed by 64 hex characters)"
                    .to_string(),
            ));
        }

        // Validate wallet address format
        if !self.wallet_address.starts_with("0x") || self.wallet_address.len() != 42 {
            return Err(ConfigError::ValidationError(
                "WALLET_ADDRESS must be a valid Ethereum address (0x followed by 40 hex characters)".to_string()
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_validation() {
        let mut config = BotConfig {
            database: DatabaseConfig {
                url: "postgres://postgres:postgres@localhost:5432/polymarket".to_string(),
            },
            gamma_api: GammaApiConfig {
                base_url: "https://gamma-api.polymarket.com".to_string(),
                sync_interval_secs: 30,
                initial_sync_on_startup: true,
                filters: GammaFilters {
                    min_liquidity: Some(100.0),
                },
            },
            trading: TradingConfig {
                probability_threshold: 0.98,
                seconds_before_resolution: 10,
                bet_amount_usd: 50.0,
            },
            risk: RiskConfig {
                max_concurrent_positions: 10,
                max_bet_per_market: 100.0,
                daily_loss_limit: 500.0,
                min_profit_cents: 50.0,
            },
            polymarket: PolymarketConfig {
                clob_url: "https://clob.polymarket.com".to_string(),
                ws_url: "wss://ws-subscriptions-clob.polymarket.com/ws/".to_string(),
                chain_id: 137,
                signature_type: 0,
            },
            scanner: ScannerConfig {
                poll_interval_secs: 30,
                min_resolution_window_mins: 60,
            },
            private_key: "0x1234567890123456789012345678901234567890123456789012345678901234"
                .to_string(),
            wallet_address: "0x1234567890123456789012345678901234567890".to_string(),
        };

        assert!(config.validate().is_ok());

        // Test invalid probability
        config.trading.probability_threshold = 1.5;
        assert!(config.validate().is_err());
        config.trading.probability_threshold = 0.98;

        // Test invalid private key
        config.private_key = "invalid".to_string();
        assert!(config.validate().is_err());
    }
}
