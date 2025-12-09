//! Oracle WebSocket message types
//!
//! Defines message structures for ChainLink and Binance oracle price feeds
//! from the Polymarket live data WebSocket.

use serde::{Deserialize, Serialize};

/// Oracle source type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OracleType {
    ChainLink,
    Binance,
}

impl OracleType {
    /// Get the subscription topic for this oracle type
    pub fn topic(&self) -> &'static str {
        match self {
            OracleType::ChainLink => "crypto_prices_chainlink",
            OracleType::Binance => "crypto_prices",
        }
    }
}

impl std::fmt::Display for OracleType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OracleType::ChainLink => write!(f, "ChainLink"),
            OracleType::Binance => write!(f, "Binance"),
        }
    }
}

/// Subscription entry for a single topic
#[derive(Debug, Clone, Serialize)]
pub struct SubscriptionEntry {
    pub topic: String,
    #[serde(rename = "type")]
    pub msg_type: String,
}

/// Subscription message sent after connecting
#[derive(Debug, Clone, Serialize)]
pub struct OracleSubscription {
    pub action: String,
    pub subscriptions: Vec<SubscriptionEntry>,
}

impl OracleSubscription {
    /// Create a new subscription for the given oracle type
    pub fn new(oracle_type: OracleType) -> Self {
        Self {
            action: "subscribe".to_string(),
            subscriptions: vec![SubscriptionEntry {
                topic: oracle_type.topic().to_string(),
                msg_type: "update".to_string(),
            }],
        }
    }
}

/// Payload containing price data
#[derive(Debug, Clone, Deserialize)]
pub struct OraclePricePayload {
    pub symbol: String,
    pub timestamp: u64,
    pub value: f64,
}

/// Price update message from WebSocket
#[derive(Debug, Clone, Deserialize)]
pub struct OraclePriceUpdate {
    pub topic: String,
    #[serde(rename = "type")]
    pub msg_type: String,
    pub timestamp: u64,
    pub payload: OraclePricePayload,
}

/// Union type for all oracle messages
#[derive(Debug)]
pub enum OracleMessage {
    /// Price update from an oracle
    PriceUpdate(OraclePriceUpdate),
    /// Pong response to heartbeat
    Pong,
    /// Unknown or unparseable message
    Unknown(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_oracle_type_topic() {
        assert_eq!(OracleType::ChainLink.topic(), "crypto_prices_chainlink");
        assert_eq!(OracleType::Binance.topic(), "crypto_prices");
    }

    #[test]
    fn test_subscription_serialization() {
        let sub = OracleSubscription::new(OracleType::ChainLink);
        let json = serde_json::to_string(&sub).unwrap();
        assert!(json.contains("crypto_prices_chainlink"));
        assert!(json.contains("subscribe"));
        assert!(json.contains("update"));
    }

    #[test]
    fn test_parse_chainlink_message() {
        let json = r#"{
            "topic": "crypto_prices_chainlink",
            "type": "update",
            "timestamp": 1753314064237,
            "payload": {
                "symbol": "eth/usd",
                "timestamp": 1753314064213,
                "value": 3456.78
            }
        }"#;

        let msg: OraclePriceUpdate = serde_json::from_str(json).unwrap();
        assert_eq!(msg.topic, "crypto_prices_chainlink");
        assert_eq!(msg.payload.symbol, "eth/usd");
        assert!((msg.payload.value - 3456.78).abs() < 0.001);
    }

    #[test]
    fn test_parse_binance_message() {
        let json = r#"{
            "topic": "crypto_prices",
            "type": "update",
            "timestamp": 1753314064237,
            "payload": {
                "symbol": "solusdt",
                "timestamp": 1753314064213,
                "value": 189.55
            }
        }"#;

        let msg: OraclePriceUpdate = serde_json::from_str(json).unwrap();
        assert_eq!(msg.topic, "crypto_prices");
        assert_eq!(msg.payload.symbol, "solusdt");
        assert!((msg.payload.value - 189.55).abs() < 0.001);
    }
}
