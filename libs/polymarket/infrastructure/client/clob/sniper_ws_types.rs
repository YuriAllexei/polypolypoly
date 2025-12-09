//! WebSocket message types for market sniper orderbook tracking
//!
//! Message types for the Polymarket market WebSocket channel:
//! - book: Initial orderbook snapshot
//! - price_change: Incremental orderbook updates
//! - tick_size_change: Tick size changes (price reaches limits)
//! - last_trade_price: Trade execution events

use serde::{Deserialize, Serialize};
use super::types::PriceLevel;

/// Subscription message to send after connecting
#[derive(Debug, Clone, Serialize)]
pub struct MarketSubscription {
    pub assets_ids: Vec<String>,
    #[serde(rename = "type")]
    pub msg_type: String,
}

impl MarketSubscription {
    pub fn new(token_ids: Vec<String>) -> Self {
        Self {
            assets_ids: token_ids,
            msg_type: "market".to_string(),
        }
    }
}

/// Initial orderbook snapshot received after subscription
/// The server sends an array of these, one per asset (Yes/No)
#[derive(Debug, Clone, Deserialize)]
pub struct BookSnapshot {
    pub market: String,
    pub asset_id: String,
    pub bids: Vec<PriceLevel>,
    pub asks: Vec<PriceLevel>,
    pub event_type: String,
    #[serde(default)]
    pub timestamp: Option<String>,
    #[serde(default)]
    pub hash: Option<String>,
    #[serde(default)]
    pub last_trade_price: Option<String>,
}

impl BookSnapshot {
    /// Get best bid (highest price)
    pub fn best_bid(&self) -> Option<&PriceLevel> {
        self.bids.first()
    }

    /// Get best ask (lowest price) - asks are sorted high to low, so last is best
    pub fn best_ask(&self) -> Option<&PriceLevel> {
        self.asks.last()
    }
}

/// Price change event received for orderbook updates
#[derive(Debug, Clone, Deserialize)]
pub struct PriceChangeEvent {
    pub market: String,
    pub price_changes: Vec<PriceChange>,
    pub timestamp: String,
    pub event_type: String,
}

/// Individual price change within an event
#[derive(Debug, Clone, Deserialize)]
pub struct PriceChange {
    pub asset_id: String,
    pub price: String,
    pub size: String,
    pub side: String,
    #[serde(default)]
    pub hash: Option<String>,
    pub best_bid: String,
    pub best_ask: String,
}

/// Tick size change event - emitted when book price reaches limits (>0.96 or <0.04)
#[derive(Debug, Clone, Deserialize)]
pub struct TickSizeChangeEvent {
    pub event_type: String,
    pub asset_id: String,
    pub market: String,
    pub old_tick_size: String,
    pub new_tick_size: String,
    #[serde(default)]
    pub side: Option<String>,
    pub timestamp: String,
}

/// Last trade price event - emitted when a maker and taker order are matched
#[derive(Debug, Clone, Deserialize)]
pub struct LastTradePriceEvent {
    pub event_type: String,
    pub asset_id: String,
    pub market: String,
    pub price: String,
    pub size: String,
    pub side: String,
    pub timestamp: String,
    #[serde(default)]
    pub fee_rate_bps: Option<String>,
}

/// Union type for all incoming WebSocket messages
#[derive(Debug)]
pub enum SniperMessage {
    /// Initial orderbook snapshots (array of BookSnapshot)
    BookSnapshots(Vec<BookSnapshot>),
    /// Price change update
    PriceChange(PriceChangeEvent),
    /// Tick size change event
    TickSizeChange(TickSizeChangeEvent),
    /// Last trade price event
    LastTradePrice(LastTradePriceEvent),
    /// Pong response to our ping
    Pong,
    /// Unknown/unhandled message
    Unknown(String),
}
