//! Strategies configuration
//!
//! Configuration for the pluggable strategy system.

use super::{ConfigError, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use tracing::info;

use crate::application::strategies::inventory_mm::InventoryMMConfig;

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

    /// Market Merger strategy configuration
    #[serde(default)]
    pub market_merger: MarketMergerConfig,

    /// Inventory MM strategy configuration
    #[serde(default)]
    pub inventory_mm: InventoryMMConfig,
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

// =============================================================================
// Market Merger Configuration
// =============================================================================

// Market Merger defaults
fn default_mm_assets() -> Vec<String> {
    vec!["BTC".to_string(), "ETH".to_string()]
}

fn default_mm_timeframes() -> Vec<String> {
    vec!["1H".to_string(), "4H".to_string()]
}

fn default_mm_poll_interval() -> f64 {
    60.0
}

fn default_mm_num_levels() -> u8 {
    3
}

fn default_mm_level_spreads() -> Vec<f64> {
    vec![1.0, 3.0, 5.0]
}

fn default_mm_quote_refresh_ms() -> u64 {
    1000
}

fn default_mm_min_profit_margin() -> f64 {
    0.02
}

fn default_mm_bootstrap_threshold() -> f64 {
    100.0
}

fn default_mm_confirmed_threshold() -> f64 {
    500.0
}

fn default_mm_bootstrap_size_pct() -> f64 {
    0.01
}

fn default_mm_confirmed_size_pct() -> f64 {
    0.03
}

fn default_mm_scaled_size_pct() -> f64 {
    0.05
}

fn default_mm_max_quote_size() -> f64 {
    200.0
}

fn default_mm_min_opportunity_score() -> f64 {
    10.0
}

fn default_mm_max_taker_size() -> f64 {
    100.0
}

fn default_mm_profit_margin_weight() -> f64 {
    100.0
}

fn default_mm_price_vs_bid_weight() -> f64 {
    200.0
}

fn default_mm_delta_coverage_weight() -> f64 {
    15.0
}

fn default_mm_avg_improvement_weight() -> f64 {
    50.0
}

fn default_mm_spread_adjust_threshold() -> f64 {
    0.10
}

fn default_mm_max_imbalance_halt() -> f64 {
    0.50
}

fn default_mm_min_merge_pairs() -> u64 {
    10
}

fn default_mm_max_merge_imbalance() -> f64 {
    0.05
}

fn default_mm_max_cost_spread() -> f64 {
    0.03
}

fn default_mm_merge_profit_threshold() -> f64 {
    0.98
}

/// Configuration for the Market Merger strategy
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketMergerConfig {
    // === Market Selection ===
    /// Crypto assets to trade (e.g., ["BTC", "ETH", "SOL", "XRP"])
    #[serde(default = "default_mm_assets")]
    pub assets: Vec<String>,

    /// Timeframes to trade (e.g., ["1H", "4H"])
    #[serde(default = "default_mm_timeframes")]
    pub timeframes: Vec<String>,

    /// Poll interval for new markets (seconds)
    #[serde(default = "default_mm_poll_interval")]
    pub poll_interval_secs: f64,

    // === Quote Ladder ===
    /// Number of bid levels per token
    #[serde(default = "default_mm_num_levels")]
    pub num_levels: u8,

    /// Spread from best bid for each level (in cents)
    #[serde(default = "default_mm_level_spreads")]
    pub level_spreads_cents: Vec<f64>,

    /// Quote refresh interval (milliseconds)
    #[serde(default = "default_mm_quote_refresh_ms")]
    pub quote_refresh_ms: u64,

    /// Minimum profit margin to maintain (buffer for fees)
    #[serde(default = "default_mm_min_profit_margin")]
    pub min_profit_margin: f64,

    // === Dynamic Sizing (phases) ===
    /// Position value threshold for Bootstrap -> Confirmed ($)
    #[serde(default = "default_mm_bootstrap_threshold")]
    pub bootstrap_threshold_usd: f64,

    /// Position value threshold for Confirmed -> Scaled ($)
    #[serde(default = "default_mm_confirmed_threshold")]
    pub confirmed_threshold_usd: f64,

    /// Size percentage in Bootstrap phase
    #[serde(default = "default_mm_bootstrap_size_pct")]
    pub bootstrap_size_pct: f64,

    /// Size percentage in Confirmed phase
    #[serde(default = "default_mm_confirmed_size_pct")]
    pub confirmed_size_pct: f64,

    /// Size percentage in Scaled phase
    #[serde(default = "default_mm_scaled_size_pct")]
    pub scaled_size_pct: f64,

    /// Maximum quote size per level ($)
    #[serde(default = "default_mm_max_quote_size")]
    pub max_quote_size_usd: f64,

    // === Opportunity-Based Taker ===
    /// Minimum opportunity score to execute taker
    #[serde(default = "default_mm_min_opportunity_score")]
    pub min_opportunity_score: f64,

    /// Maximum taker order size (tokens)
    #[serde(default = "default_mm_max_taker_size")]
    pub max_taker_size: f64,

    /// Score weight for profit margin (points per 1% margin)
    #[serde(default = "default_mm_profit_margin_weight")]
    pub profit_margin_weight: f64,

    /// Score weight for price vs our bid (points per cent below bid)
    #[serde(default = "default_mm_price_vs_bid_weight")]
    pub price_vs_bid_weight: f64,

    /// Score weight for delta coverage (points for 100% coverage)
    #[serde(default = "default_mm_delta_coverage_weight")]
    pub delta_coverage_weight: f64,

    /// Score weight for avg cost improvement (points per cent below avg)
    #[serde(default = "default_mm_avg_improvement_weight")]
    pub avg_improvement_weight: f64,

    // === Spread Skew (for bid adjustment) ===
    /// Imbalance threshold to start adjusting spreads
    #[serde(default = "default_mm_spread_adjust_threshold")]
    pub spread_adjust_threshold: f64,

    /// Imbalance threshold to halt quoting on overweight side
    #[serde(default = "default_mm_max_imbalance_halt")]
    pub max_imbalance_halt: f64,

    // === Merge Conditions ===
    /// Minimum pairs to trigger merge
    #[serde(default = "default_mm_min_merge_pairs")]
    pub min_merge_pairs: u64,

    /// Maximum imbalance to allow merge
    #[serde(default = "default_mm_max_merge_imbalance")]
    pub max_merge_imbalance: f64,

    /// Maximum cost spread between Up and Down avg costs
    #[serde(default = "default_mm_max_cost_spread")]
    pub max_cost_spread: f64,

    /// Combined cost threshold for merge (must be below this)
    #[serde(default = "default_mm_merge_profit_threshold")]
    pub merge_profit_threshold: f64,
}

impl Default for MarketMergerConfig {
    fn default() -> Self {
        Self {
            assets: default_mm_assets(),
            timeframes: default_mm_timeframes(),
            poll_interval_secs: default_mm_poll_interval(),
            num_levels: default_mm_num_levels(),
            level_spreads_cents: default_mm_level_spreads(),
            quote_refresh_ms: default_mm_quote_refresh_ms(),
            min_profit_margin: default_mm_min_profit_margin(),
            bootstrap_threshold_usd: default_mm_bootstrap_threshold(),
            confirmed_threshold_usd: default_mm_confirmed_threshold(),
            bootstrap_size_pct: default_mm_bootstrap_size_pct(),
            confirmed_size_pct: default_mm_confirmed_size_pct(),
            scaled_size_pct: default_mm_scaled_size_pct(),
            max_quote_size_usd: default_mm_max_quote_size(),
            min_opportunity_score: default_mm_min_opportunity_score(),
            max_taker_size: default_mm_max_taker_size(),
            profit_margin_weight: default_mm_profit_margin_weight(),
            price_vs_bid_weight: default_mm_price_vs_bid_weight(),
            delta_coverage_weight: default_mm_delta_coverage_weight(),
            avg_improvement_weight: default_mm_avg_improvement_weight(),
            spread_adjust_threshold: default_mm_spread_adjust_threshold(),
            max_imbalance_halt: default_mm_max_imbalance_halt(),
            min_merge_pairs: default_mm_min_merge_pairs(),
            max_merge_imbalance: default_mm_max_merge_imbalance(),
            max_cost_spread: default_mm_max_cost_spread(),
            merge_profit_threshold: default_mm_merge_profit_threshold(),
        }
    }
}

impl MarketMergerConfig {
    fn validate(&self) -> Result<()> {
        if self.poll_interval_secs <= 0.0 {
            return Err(ConfigError::ValidationError(
                "market_merger.poll_interval_secs must be greater than 0".to_string(),
            ));
        }
        if self.assets.is_empty() {
            return Err(ConfigError::ValidationError(
                "market_merger.assets cannot be empty".to_string(),
            ));
        }
        if self.timeframes.is_empty() {
            return Err(ConfigError::ValidationError(
                "market_merger.timeframes cannot be empty".to_string(),
            ));
        }
        Ok(())
    }

    /// Get the spread for a given level (in cents)
    pub fn spread_for_level(&self, level: u8) -> f64 {
        self.level_spreads_cents
            .get(level as usize)
            .copied()
            .unwrap_or(5.0)
    }

    /// Get the size multiplier for a given level
    pub fn size_multiplier_for_level(&self, level: u8) -> f64 {
        match level {
            0 => 1.0,
            1 => 1.5,
            2 => 2.0,
            _ => 2.0,
        }
    }

    /// Check if a crypto asset is configured for trading
    pub fn is_asset_enabled(&self, asset: &str) -> bool {
        self.assets.iter().any(|a| a.eq_ignore_ascii_case(asset))
    }

    /// Check if a timeframe is configured for trading
    pub fn is_timeframe_enabled(&self, timeframe: &str) -> bool {
        self.timeframes
            .iter()
            .any(|t| t.eq_ignore_ascii_case(timeframe))
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
            market_merger: MarketMergerConfig::default(),
            inventory_mm: InventoryMMConfig::default(),
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

        // Validate market_merger config
        self.market_merger.validate()?;

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
        info!("Market Merger Strategy:");
        info!("  Assets: {:?}", self.market_merger.assets);
        info!("  Timeframes: {:?}", self.market_merger.timeframes);
        info!(
            "  Poll interval: {} seconds",
            self.market_merger.poll_interval_secs
        );
        info!("  Num levels: {}", self.market_merger.num_levels);
        info!(
            "  Min profit margin: ${:.2}",
            self.market_merger.min_profit_margin
        );
        info!("Inventory MM Strategy:");
        info!("  Markets: {:?}", self.inventory_mm.markets);
        info!(
            "  Poll interval: {} seconds",
            self.inventory_mm.poll_interval_secs
        );
        info!(
            "  Tick interval: {} ms",
            self.inventory_mm.tick_interval_ms
        );
        info!("  Solver config (4-layer quoter):");
        info!("    Num levels: {}", self.inventory_mm.solver.num_levels);
        info!("    Order size: {:.1}", self.inventory_mm.solver.order_size);
        info!("    Base spread: {:.3}", self.inventory_mm.solver.base_spread);
        info!("    Max imbalance: {:.1}%", self.inventory_mm.solver.max_imbalance * 100.0);
        info!("    Max position: {:.1} (0=unlimited)", self.inventory_mm.solver.max_position);
        info!("    Oracle sensitivity: {:.1}", self.inventory_mm.solver.oracle_sensitivity);
        info!("    Gamma inv: {:.1}", self.inventory_mm.solver.gamma_inv);
        info!("    Lambda size: {:.1}", self.inventory_mm.solver.lambda_size);
        info!("    Time decay (min): {:.1}", self.inventory_mm.solver.time_decay_minutes);
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
