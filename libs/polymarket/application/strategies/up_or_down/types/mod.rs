//! Type definitions for the Up or Down strategy.

mod market_metadata;
mod tracker;

pub use market_metadata::{
    CryptoAsset, OracleSource, Timeframe, FINAL_SECONDS_BYPASS, GUARDIAN_SAFETY_BPS,
    MAX_RECONNECT_ATTEMPTS, REQUIRED_TAGS, STALENESS_THRESHOLD_SECS,
};
pub use tracker::{
    MarketTrackerContext, OrderbookCheckResult, OrderInfo, TrackerState, TrackingLoopExit,
};
