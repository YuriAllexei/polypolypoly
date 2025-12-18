//! Tracker types for the Up or Down strategy.
//!
//! Contains the context, state, and result types used during market tracking.

use super::market_metadata::{CryptoAsset, OracleSource, Timeframe};
use crate::domain::DbMarket;
use crate::infrastructure::config::UpOrDownConfig;
use chrono::{DateTime, Utc};
use std::collections::{HashMap, HashSet};
use std::time::Instant;

// =============================================================================
// Market Tracker Context
// =============================================================================

/// Context holding immutable market information for the tracker
pub struct MarketTrackerContext {
    pub market_id: String,
    pub market_question: String,
    pub market_url: String,
    pub oracle_source: OracleSource,
    pub crypto_asset: CryptoAsset,
    pub timeframe: Timeframe,
    pub token_ids: Vec<String>,
    pub outcome_map: HashMap<String, String>,
    /// Market end time for dynamic threshold calculation
    pub market_end_time: DateTime<Utc>,
    /// Minimum threshold in seconds (when close to market end)
    pub threshold_min: f64,
    /// Maximum threshold in seconds (when far from market end)
    pub threshold_max: f64,
    /// Decay time constant in seconds
    pub threshold_tau: f64,
    /// The opening price that determines "up" or "down" outcome
    pub price_to_beat: Option<f64>,
    /// Oracle price difference threshold in basis points
    pub oracle_bps_price_threshold: f64,
}

impl MarketTrackerContext {
    /// Create a new tracker context from market data and config
    pub fn new(
        market: &DbMarket,
        config: &UpOrDownConfig,
        outcomes: Vec<String>,
    ) -> anyhow::Result<Self> {
        let tags = market
            .parse_tags()
            .unwrap_or(serde_json::Value::Array(vec![]));
        let token_ids = market.parse_token_ids()?;

        // Build outcome map (token_id -> outcome name)
        let outcome_map: HashMap<String, String> = token_ids
            .iter()
            .zip(outcomes.iter())
            .map(|(id, outcome)| (id.clone(), outcome.clone()))
            .collect();

        let market_url = market
            .slug
            .as_ref()
            .map(|s| format!("https://polymarket.com/event/{}", s))
            .unwrap_or_else(|| "N/A".to_string());

        // Parse market end time
        let market_end_time = DateTime::parse_from_rfc3339(&market.end_date)
            .map_err(|e| anyhow::anyhow!("Failed to parse market end_date: {}", e))?
            .with_timezone(&Utc);

        Ok(Self {
            market_id: market.id.clone(),
            market_question: market.question.clone(),
            market_url,
            oracle_source: OracleSource::from_description(&market.description),
            crypto_asset: CryptoAsset::from_tags(&tags),
            timeframe: Timeframe::from_tags(&tags),
            token_ids,
            outcome_map,
            market_end_time,
            threshold_min: config.threshold_min,
            threshold_max: config.threshold_max,
            threshold_tau: config.threshold_tau,
            price_to_beat: None,
            oracle_bps_price_threshold: config.oracle_bps_price_threshold,
        })
    }

    /// Get the outcome name for a token ID
    pub fn get_outcome_name(&self, token_id: &str) -> String {
        self.outcome_map
            .get(token_id)
            .cloned()
            .unwrap_or_else(|| "Unknown".to_string())
    }

    /// Set the price to beat (opening price from API)
    pub fn set_price_to_beat(&mut self, price: Option<f64>) {
        self.price_to_beat = price;
    }

    /// Format the price to beat for display
    pub fn format_price_to_beat(&self) -> String {
        match self.price_to_beat {
            Some(price) => format!("${:.2}", price),
            None => "N/A".to_string(),
        }
    }
}

// =============================================================================
// Tracker State
// =============================================================================

/// Mutable state for the market tracker
pub struct TrackerState {
    /// Timers tracking how long each token has had no asks
    pub no_asks_timers: HashMap<String, Instant>,
    /// Tokens that have exceeded the no-asks threshold
    pub threshold_triggered: HashSet<String>,
    /// Orders placed: token_id -> order_id (for cancellation tracking)
    pub order_placed: HashMap<String, String>,
}

impl TrackerState {
    /// Create a new empty tracker state
    pub fn new() -> Self {
        Self {
            no_asks_timers: HashMap::new(),
            threshold_triggered: HashSet::new(),
            order_placed: HashMap::new(),
        }
    }

    /// Get all order IDs for cancellation
    pub fn get_order_ids(&self) -> Vec<String> {
        self.order_placed.values().cloned().collect()
    }

    /// Clear timer state (used on reconnection)
    pub fn clear_timers(&mut self) {
        self.no_asks_timers.clear();
        self.threshold_triggered.clear();
    }
}

impl Default for TrackerState {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Result Enums
// =============================================================================

/// Result of checking orderbook state for a single token
pub enum OrderbookCheckResult {
    /// Asks exist - market is active
    HasAsks,
    /// No asks - timer started or continuing
    NoAsks,
    /// No asks and threshold exceeded - should place order
    ThresholdExceeded { elapsed_secs: f64 },
}

/// Reason for exiting the tracking loop
#[derive(Debug)]
pub enum TrackingLoopExit {
    Shutdown,
    MarketEnded,
    AllOrderbooksEmpty,
    WebSocketDisconnected,
    StaleOrderbook,
}

impl TrackingLoopExit {
    /// Get a string description of the exit reason
    pub fn as_str(&self) -> &'static str {
        match self {
            TrackingLoopExit::Shutdown => "shutdown",
            TrackingLoopExit::MarketEnded => "market_ended",
            TrackingLoopExit::AllOrderbooksEmpty => "all_empty",
            TrackingLoopExit::WebSocketDisconnected => "ws_disconnected",
            TrackingLoopExit::StaleOrderbook => "stale_orderbook",
        }
    }

    /// Check if this exit reason allows reconnection
    pub fn should_reconnect(&self) -> bool {
        matches!(
            self,
            TrackingLoopExit::StaleOrderbook | TrackingLoopExit::WebSocketDisconnected
        )
    }
}
