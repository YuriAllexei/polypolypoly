//! WebSocket message types for market sniper orderbook tracking

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

/// Union type for all incoming WebSocket messages
#[derive(Debug)]
pub enum SniperMessage {
    /// Initial orderbook snapshots (array of BookSnapshot)
    BookSnapshots(Vec<BookSnapshot>),
    /// Price change update
    PriceChange(PriceChangeEvent),
    /// Pong response to our ping
    Pong,
    /// Unknown/unhandled message
    Unknown(String),
}
