// ! Strategy domain entities and errors
//!
//! Contains business entities and errors for trading strategies

use chrono::{DateTime, Utc};
use thiserror::Error;

// ==================== ERRORS ====================

#[derive(Error, Debug)]
pub enum ExecutorError {
    #[error("REST API error: {0}")]
    RestError(String),

    #[error("Risk check failed: {0}")]
    RiskError(#[from] RiskError),

    #[error("No profitable opportunity found")]
    NoOpportunity,

    #[error("Orderbook empty or invalid")]
    InvalidOrderbook,
}

impl ExecutorError {
    /// Create ExecutorError from any error
    pub fn from_rest_error<E: std::fmt::Display>(err: E) -> Self {
        ExecutorError::RestError(err.to_string())
    }
}

#[derive(Error, Debug)]
pub enum RiskError {
    #[error("Max concurrent positions limit reached ({0})")]
    MaxPositionsReached(usize),

    #[error("Bet amount ${0} exceeds max bet per market ${1}")]
    MaxBetExceeded(f64, f64),

    #[error("Daily loss limit reached: -${0}")]
    DailyLossLimitReached(f64),

    #[error("Trading is halted due to risk limits")]
    TradingHalted,
}

// ==================== ENTITIES ====================

/// Information about a market being monitored
#[derive(Debug, Clone)]
pub struct MonitoredMarket {
    pub market_id: String,
    pub question: String,
    pub resolution_time: DateTime<Utc>,
    pub token_ids: Vec<String>,  // Outcome token IDs
}

/// Executed trade information
#[derive(Debug, Clone)]
pub struct ExecutedTrade {
    pub market_id: String,
    pub token_id: String,
    pub side: String,  // "Buy" or "Sell"
    pub amount_usd: f64,
    pub price: f64,
    pub expected_profit: f64,
    pub order_id: String,
}

/// Trading configuration
#[derive(Debug, Clone)]
pub struct TradingConfig {
    pub probability_threshold: f64,
    pub bet_amount_usd: f64,
}

/// Risk management configuration
#[derive(Debug, Clone)]
pub struct RiskConfig {
    pub max_concurrent_positions: usize,
    pub max_bet_per_market: f64,
    pub daily_loss_limit: f64,
    pub min_profit_cents: f64,
}

/// Daily statistics for risk management
#[derive(Debug, Clone)]
pub struct DailyStats {
    pub trades: usize,
    pub wins: usize,
    pub losses: usize,
    pub pnl: f64,
    pub win_rate: f64,
}

impl DailyStats {
    pub fn new(trades: usize, wins: usize, pnl: f64) -> Self {
        let losses = trades.saturating_sub(wins);
        let win_rate = if trades > 0 {
            (wins as f64) / (trades as f64) * 100.0
        } else {
            0.0
        };

        Self {
            trades,
            wins,
            losses,
            pnl,
            win_rate,
        }
    }
}
