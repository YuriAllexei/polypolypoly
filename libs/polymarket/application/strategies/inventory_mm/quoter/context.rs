//! Quoter context and market information.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use chrono::{DateTime, Utc};

use crate::infrastructure::{SharedOrderState, SharedPositionTracker, SharedOraclePrices};
use crate::infrastructure::client::clob::TradingClient;

/// Information about a specific market that a Quoter is managing.
#[derive(Debug, Clone)]
pub struct MarketInfo {
    /// Unique market identifier
    pub market_id: String,
    /// Condition ID for merging tokens
    pub condition_id: String,
    /// Token ID for UP outcome
    pub up_token_id: String,
    /// Token ID for DOWN outcome
    pub down_token_id: String,
    /// Market end time
    pub end_time: DateTime<Utc>,
    /// Symbol (e.g., "BTC", "ETH")
    pub symbol: String,
    /// Timeframe (e.g., "15m", "1hr")
    pub timeframe: String,
    /// Price threshold (price_to_beat) for the market question
    /// e.g., $97,000 for "Will BTC be above $97,000?"
    pub threshold: f64,
}

impl MarketInfo {
    pub fn new(
        market_id: String,
        condition_id: String,
        up_token_id: String,
        down_token_id: String,
        end_time: DateTime<Utc>,
        symbol: String,
        timeframe: String,
        threshold: f64,
    ) -> Self {
        Self {
            market_id,
            condition_id,
            up_token_id,
            down_token_id,
            end_time,
            symbol,
            timeframe,
            threshold,
        }
    }

    /// Check if the market has expired.
    pub fn is_expired(&self) -> bool {
        Utc::now() >= self.end_time
    }

    /// Get a short description for logging.
    pub fn short_desc(&self) -> String {
        format!("{} {} ({})", self.symbol, self.timeframe, &self.market_id[..8.min(self.market_id.len())])
    }
}

/// Shared state passed to each quoter (all Clone-able).
/// This bundles all the shared infrastructure that quoters need.
///
/// NOTE: Each quoter spawns its own Executor thread for order execution.
/// This ensures markets are independent and don't block each other.
#[derive(Clone)]
pub struct QuoterContext {
    /// Trading client for order execution (shared, has connection pooling)
    pub trading: Arc<TradingClient>,
    /// Shared order state from user WebSocket
    pub order_state: SharedOrderState,
    /// Shared position tracker
    pub position_tracker: SharedPositionTracker,
    /// Shutdown flag for graceful termination
    pub shutdown_flag: Arc<AtomicBool>,
    /// Shared oracle prices (ChainLink + Binance feeds)
    pub oracle_prices: SharedOraclePrices,
}

impl QuoterContext {
    pub fn new(
        trading: Arc<TradingClient>,
        order_state: SharedOrderState,
        position_tracker: SharedPositionTracker,
        shutdown_flag: Arc<AtomicBool>,
        oracle_prices: SharedOraclePrices,
    ) -> Self {
        Self {
            trading,
            order_state,
            position_tracker,
            shutdown_flag,
            oracle_prices,
        }
    }

    pub fn is_running(&self) -> bool {
        self.shutdown_flag.load(std::sync::atomic::Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_market_info_is_expired() {
        let past = Utc::now() - chrono::Duration::hours(1);
        let future = Utc::now() + chrono::Duration::hours(1);

        let expired_market = MarketInfo::new(
            "market-1".to_string(),
            "cond-1".to_string(),
            "up-1".to_string(),
            "down-1".to_string(),
            past,
            "BTC".to_string(),
            "15m".to_string(),
            97000.0,
        );
        assert!(expired_market.is_expired());

        let active_market = MarketInfo::new(
            "market-2".to_string(),
            "cond-2".to_string(),
            "up-2".to_string(),
            "down-2".to_string(),
            future,
            "ETH".to_string(),
            "1hr".to_string(),
            3500.0,
        );
        assert!(!active_market.is_expired());
    }

    #[test]
    fn test_market_info_short_desc() {
        let market = MarketInfo::new(
            "0x1234567890abcdef".to_string(),
            "cond-1".to_string(),
            "up-1".to_string(),
            "down-1".to_string(),
            Utc::now(),
            "BTC".to_string(),
            "15m".to_string(),
            97000.0,
        );
        assert_eq!(market.short_desc(), "BTC 15m (0x123456)");
    }
}
