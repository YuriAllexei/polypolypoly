//! User Channel WebSocket Message Types
//!
//! Types for parsing messages from the Polymarket user WebSocket channel.
//! See: https://docs.polymarket.com/developers/CLOB/websocket/user-channel

use serde::{Deserialize, Serialize};

// =============================================================================
// Subscription / Authentication
// =============================================================================

/// Subscription message for the user channel
#[derive(Debug, Clone, Serialize)]
pub struct UserSubscription {
    #[serde(rename = "type")]
    pub msg_type: String,
    pub auth: AuthPayload,
}

impl UserSubscription {
    pub fn new(api_key: String, secret: String, passphrase: String) -> Self {
        Self {
            msg_type: "user".to_string(),
            auth: AuthPayload {
                api_key,
                secret,
                passphrase,
            },
        }
    }
}

/// Authentication payload for user channel subscription
#[derive(Debug, Clone, Serialize)]
pub struct AuthPayload {
    #[serde(rename = "apiKey")]
    pub api_key: String,
    pub secret: String,
    pub passphrase: String,
}

// =============================================================================
// Trade Message
// =============================================================================

/// Trade event from the user channel
///
/// Received when:
/// - Market order is matched (MATCHED)
/// - Limit order is included in a trade (MATCHED)
/// - Trade status changes (MINED, CONFIRMED, RETRYING, FAILED)
#[derive(Debug, Clone, Deserialize)]
pub struct TradeMessage {
    /// Token ID of the order
    pub asset_id: String,

    /// Event type - always "trade"
    pub event_type: String,

    /// Trade identifier
    pub id: String,

    /// Timestamp of last modification
    #[serde(default)]
    pub last_update: Option<String>,

    /// Array of maker order details
    #[serde(default)]
    pub maker_orders: Vec<MakerOrder>,

    /// Condition ID (market identifier)
    pub market: String,

    /// Trade execution timestamp
    #[serde(default)]
    pub matchtime: Option<String>,

    /// YES or NO outcome
    pub outcome: String,

    /// API key of event owner
    pub owner: String,

    /// Trade price
    pub price: String,

    /// BUY or SELL
    pub side: String,

    /// Trade quantity
    pub size: String,

    /// Current trade status: MATCHED, MINED, CONFIRMED, RETRYING, FAILED
    pub status: String,

    /// Taker's order identifier
    #[serde(default)]
    pub taker_order_id: Option<String>,

    /// Event timestamp
    pub timestamp: String,

    /// API key of trade owner
    #[serde(default)]
    pub trade_owner: Option<String>,

    /// Message type - always "TRADE"
    #[serde(rename = "type")]
    pub msg_type: String,
}

/// Maker order details within a trade
#[derive(Debug, Clone, Deserialize)]
pub struct MakerOrder {
    pub asset_id: String,
    pub matched_amount: String,
    pub order_id: String,
    pub outcome: String,
    pub owner: String,
    pub price: String,
}

// =============================================================================
// Order Message
// =============================================================================

/// Order event from the user channel
///
/// Received when:
/// - Order is placed (PLACEMENT)
/// - Order is updated/partially filled (UPDATE)
/// - Order is cancelled (CANCELLATION)
#[derive(Debug, Clone, Deserialize)]
pub struct OrderMessage {
    /// Token ID
    pub asset_id: String,

    /// Referenced trade IDs
    #[serde(default)]
    pub associate_trades: Vec<String>,

    /// Event type - always "order"
    pub event_type: String,

    /// Order identifier
    pub id: String,

    /// Condition ID (market identifier)
    pub market: String,

    /// Order owner's API key
    #[serde(default)]
    pub order_owner: Option<String>,

    /// Initial order quantity
    pub original_size: String,

    /// Outcome (YES/NO)
    pub outcome: String,

    /// Order owner
    pub owner: String,

    /// Order price
    pub price: String,

    /// BUY or SELL
    pub side: String,

    /// Filled quantity
    pub size_matched: String,

    /// Event timestamp
    pub timestamp: String,

    /// Message type: PLACEMENT, UPDATE, or CANCELLATION
    #[serde(rename = "type")]
    pub msg_type: String,
}

/// Order type enum for easier matching
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderType {
    Placement,
    Update,
    Cancellation,
}

impl OrderMessage {
    /// Parse the order type from the message
    pub fn order_type(&self) -> OrderType {
        match self.msg_type.as_str() {
            "PLACEMENT" => OrderType::Placement,
            "UPDATE" => OrderType::Update,
            "CANCELLATION" => OrderType::Cancellation,
            _ => OrderType::Update, // Default to update for unknown types
        }
    }
}

/// Trade status enum
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeStatus {
    Matched,
    Mined,
    Confirmed,
    Retrying,
    Failed,
}

impl TradeMessage {
    /// Parse the trade status from the message
    pub fn trade_status(&self) -> TradeStatus {
        match self.status.as_str() {
            "MATCHED" => TradeStatus::Matched,
            "MINED" => TradeStatus::Mined,
            "CONFIRMED" => TradeStatus::Confirmed,
            "RETRYING" => TradeStatus::Retrying,
            "FAILED" => TradeStatus::Failed,
            _ => TradeStatus::Matched, // Default
        }
    }
}

// =============================================================================
// Union Message Type
// =============================================================================

/// Union type for all user channel messages
#[derive(Debug, Clone)]
pub enum UserMessage {
    /// Trade event
    Trade(TradeMessage),
    /// Order event
    Order(OrderMessage),
    /// Pong response to heartbeat
    Pong,
    /// Unknown or unparseable message
    Unknown(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_subscription_serialization() {
        let sub = UserSubscription::new(
            "test-api-key".to_string(),
            "test-secret".to_string(),
            "test-passphrase".to_string(),
        );
        let json = serde_json::to_string(&sub).unwrap();
        assert!(json.contains("\"type\":\"user\""));
        assert!(json.contains("\"apiKey\":\"test-api-key\""));
    }

    #[test]
    fn test_order_message_parsing() {
        let json = r#"{
            "asset_id": "123",
            "associate_trades": [],
            "event_type": "order",
            "id": "order-1",
            "market": "market-1",
            "original_size": "100",
            "outcome": "YES",
            "owner": "owner-1",
            "price": "0.5",
            "side": "BUY",
            "size_matched": "0",
            "timestamp": "2024-01-01T00:00:00Z",
            "type": "PLACEMENT"
        }"#;
        let order: OrderMessage = serde_json::from_str(json).unwrap();
        assert_eq!(order.id, "order-1");
        assert_eq!(order.order_type(), OrderType::Placement);
    }

    #[test]
    fn test_trade_message_parsing() {
        let json = r#"{
            "asset_id": "123",
            "event_type": "trade",
            "id": "trade-1",
            "maker_orders": [],
            "market": "market-1",
            "outcome": "YES",
            "owner": "owner-1",
            "price": "0.5",
            "side": "BUY",
            "size": "10",
            "status": "MATCHED",
            "timestamp": "2024-01-01T00:00:00Z",
            "type": "TRADE"
        }"#;
        let trade: TradeMessage = serde_json::from_str(json).unwrap();
        assert_eq!(trade.id, "trade-1");
        assert_eq!(trade.trade_status(), TradeStatus::Matched);
    }
}
