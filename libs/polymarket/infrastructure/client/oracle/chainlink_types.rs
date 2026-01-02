//! ChainLink Data Streams WebSocket types
//!
//! Types and utilities for connecting to ChainLink's Data Streams WebSocket API.

use serde::Deserialize;
use std::collections::HashMap;

/// Feed ID mapping for ChainLink Data Streams
/// Maps symbol names to their corresponding feed IDs
pub struct FeedIdMap {
    symbol_to_feed: HashMap<String, String>,
    feed_to_symbol: HashMap<String, String>,
}

impl FeedIdMap {
    /// Create a new FeedIdMap with default mappings
    pub fn new() -> Self {
        let mappings = [
            ("BTC", "0x00039d9e45394f473ab1f050a1b963e6b05351e52d71e507509ada0c95ed75b8"),
            ("ETH", "0x000362205e10b3a147d02792eccee483dca6c7b44ecce7012cb8c6e0b68b3ae9"),
            ("XRP", "0x0003c16c6aed42294f5cb4741f6e59ba2d728f0eae2eb9e6d3f555808c59fc45"),
            ("SOL", "0x0003b778d3f6b2ac4991302b89cb313f99a42467d6c9c5f96f57c29c0d2bc24f"),
        ];

        let mut symbol_to_feed = HashMap::new();
        let mut feed_to_symbol = HashMap::new();

        for (symbol, feed_id) in mappings {
            symbol_to_feed.insert(symbol.to_string(), feed_id.to_lowercase());
            feed_to_symbol.insert(feed_id.to_lowercase(), symbol.to_string());
        }

        Self {
            symbol_to_feed,
            feed_to_symbol,
        }
    }

    /// Get feed ID for a symbol
    pub fn get_feed_id(&self, symbol: &str) -> Option<&String> {
        self.symbol_to_feed.get(&symbol.to_uppercase())
    }

    /// Get symbol for a feed ID
    pub fn get_symbol(&self, feed_id: &str) -> Option<&String> {
        self.feed_to_symbol.get(&feed_id.to_lowercase())
    }

    /// Get all feed IDs as comma-separated string for WebSocket URL
    pub fn get_feed_ids_param(&self) -> String {
        self.symbol_to_feed
            .values()
            .cloned()
            .collect::<Vec<_>>()
            .join(",")
    }

    /// Get all symbols
    pub fn symbols(&self) -> Vec<&String> {
        self.symbol_to_feed.keys().collect()
    }
}

impl Default for FeedIdMap {
    fn default() -> Self {
        Self::new()
    }
}

/// WebSocket message from ChainLink Data Streams
#[derive(Debug, Clone, Deserialize)]
pub struct ChainLinkWsMessage {
    pub report: ChainLinkReport,
}

/// Report wrapper from WebSocket
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainLinkReport {
    /// The feed ID (hex string)
    #[serde(rename = "feedID")]
    pub feed_id: String,
    /// The full report blob (hex encoded)
    pub full_report: String,
}

/// Parsed message types for the router
#[derive(Debug)]
pub enum ChainLinkMessage {
    /// A price report
    Report(ChainLinkWsMessage),
    /// Pong response
    Pong,
    /// Unknown or unparseable message
    Unknown(String),
}

/// Decoded price data from a report
#[derive(Debug, Clone)]
pub struct DecodedPrice {
    /// Symbol (e.g., "BTC", "ETH")
    pub symbol: String,
    /// Feed ID
    pub feed_id: String,
    /// Benchmark price in USD
    pub price: f64,
    /// Bid price
    pub bid: f64,
    /// Ask price
    pub ask: f64,
    /// Observation timestamp (unix seconds)
    pub timestamp: u64,
    /// Valid from timestamp
    pub valid_from: u64,
    /// Expires at timestamp
    pub expires_at: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feed_id_map() {
        let map = FeedIdMap::new();

        // Test symbol to feed ID
        assert_eq!(
            map.get_feed_id("BTC"),
            Some(&"0x00039d9e45394f473ab1f050a1b963e6b05351e52d71e507509ada0c95ed75b8".to_string())
        );
        assert_eq!(
            map.get_feed_id("btc"), // case insensitive
            Some(&"0x00039d9e45394f473ab1f050a1b963e6b05351e52d71e507509ada0c95ed75b8".to_string())
        );

        // Test feed ID to symbol
        assert_eq!(
            map.get_symbol("0x00039d9e45394f473ab1f050a1b963e6b05351e52d71e507509ada0c95ed75b8"),
            Some(&"BTC".to_string())
        );

        // Test unknown
        assert_eq!(map.get_feed_id("UNKNOWN"), None);
    }

    #[test]
    fn test_feed_ids_param() {
        let map = FeedIdMap::new();
        let param = map.get_feed_ids_param();

        // Should contain all feed IDs
        assert!(param.contains("0x00039d9e45394f473ab1f050a1b963e6b05351e52d71e507509ada0c95ed75b8"));
        assert!(param.contains("0x000362205e10b3a147d02792eccee483dca6c7b44ecce7012cb8c6e0b68b3ae9"));
    }

    #[test]
    fn test_parse_ws_message() {
        let json = r#"{
            "report": {
                "feedID": "0x00039d9e45394f473ab1f050a1b963e6b05351e52d71e507509ada0c95ed75b8",
                "fullReport": "0x0006..."
            }
        }"#;

        let msg: ChainLinkWsMessage = serde_json::from_str(json).unwrap();
        assert_eq!(
            msg.report.feed_id,
            "0x00039d9e45394f473ab1f050a1b963e6b05351e52d71e507509ada0c95ed75b8"
        );
    }
}
