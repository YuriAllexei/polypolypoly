//! Binance WebSocket message types
//!
//! Defines message structures for direct Binance trade streams.
//! Uses the combined stream format from wss://stream.binance.com:9443/stream

use serde::Deserialize;

// =============================================================================
// BinanceAsset - Supported crypto assets
// =============================================================================

/// Supported crypto assets for Binance price tracking
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BinanceAsset {
    BTC,
    ETH,
    SOL,
    XRP,
}

impl BinanceAsset {
    /// Get the stream name for this asset (e.g., "btcusdt@trade")
    pub fn stream_name(&self) -> &'static str {
        match self {
            BinanceAsset::BTC => "btcusdt@trade",
            BinanceAsset::ETH => "ethusdt@trade",
            BinanceAsset::SOL => "solusdt@trade",
            BinanceAsset::XRP => "xrpusdt@trade",
        }
    }

    /// Parse symbol string to asset (e.g., "BTCUSDT" -> BTC)
    pub fn from_symbol(symbol: &str) -> Option<Self> {
        let upper = symbol.to_uppercase();
        match upper.as_str() {
            "BTCUSDT" => Some(BinanceAsset::BTC),
            "ETHUSDT" => Some(BinanceAsset::ETH),
            "SOLUSDT" => Some(BinanceAsset::SOL),
            "XRPUSDT" => Some(BinanceAsset::XRP),
            _ => None,
        }
    }

    /// Get normalized symbol name (e.g., "BTC")
    pub fn symbol(&self) -> &'static str {
        match self {
            BinanceAsset::BTC => "BTC",
            BinanceAsset::ETH => "ETH",
            BinanceAsset::SOL => "SOL",
            BinanceAsset::XRP => "XRP",
        }
    }

    /// All supported assets
    pub fn all() -> &'static [BinanceAsset] {
        &[
            BinanceAsset::BTC,
            BinanceAsset::ETH,
            BinanceAsset::SOL,
            BinanceAsset::XRP,
        ]
    }
}

impl std::fmt::Display for BinanceAsset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.symbol())
    }
}

// =============================================================================
// BinanceTradeData - Raw trade data from Binance
// =============================================================================

/// Raw trade data from Binance stream
///
/// Example JSON:
/// ```json
/// {
///     "e": "trade",
///     "E": 1766482935996,
///     "s": "BTCUSDT",
///     "t": 5697810014,
///     "p": "87398.39000000",
///     "q": "0.00103000",
///     "T": 1766482935995,
///     "m": false,
///     "M": true
/// }
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct BinanceTradeData {
    /// Event type (always "trade")
    #[serde(rename = "e")]
    pub event_type: String,

    /// Event time (ms since epoch)
    #[serde(rename = "E")]
    pub event_time: u64,

    /// Symbol (e.g., "BTCUSDT")
    #[serde(rename = "s")]
    pub symbol: String,

    /// Trade ID
    #[serde(rename = "t")]
    pub trade_id: u64,

    /// Price as string (need to parse to f64)
    #[serde(rename = "p")]
    pub price: String,

    /// Quantity as string
    #[serde(rename = "q")]
    pub quantity: String,

    /// Trade time (ms since epoch)
    #[serde(rename = "T")]
    pub trade_time: u64,

    /// Is buyer the market maker (true = sell aggressor, false = buy aggressor)
    #[serde(rename = "m")]
    pub is_buyer_maker: bool,

    /// Ignore (always true for valid trades)
    #[serde(rename = "M", default)]
    pub ignore: bool,
}

impl BinanceTradeData {
    /// Parse price string to f64
    pub fn price_f64(&self) -> Option<f64> {
        self.price.parse().ok()
    }

    /// Parse quantity string to f64
    pub fn quantity_f64(&self) -> Option<f64> {
        self.quantity.parse().ok()
    }
}

// =============================================================================
// BinanceStreamWrapper - Combined stream format
// =============================================================================

/// Combined stream wrapper (Binance sends this format on /stream endpoint)
///
/// Example JSON:
/// ```json
/// {
///     "stream": "btcusdt@trade",
///     "data": { ... trade data ... }
/// }
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct BinanceStreamWrapper {
    /// Stream name (e.g., "btcusdt@trade")
    pub stream: String,

    /// Trade data payload
    pub data: BinanceTradeData,
}

// =============================================================================
// BinanceMessage - Parsed message enum for router
// =============================================================================

/// Parsed message types for the router
#[derive(Debug)]
pub enum BinanceMessage {
    /// Trade update with parsed data
    Trade(BinanceStreamWrapper),
    /// Unknown or unparseable message
    Unknown(String),
}

// =============================================================================
// BinanceRoute - Route key for message handler
// =============================================================================

/// Route key for Binance messages (all trades go to same handler)
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub enum BinanceRoute {
    Trades,
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_binance_asset_stream_name() {
        assert_eq!(BinanceAsset::BTC.stream_name(), "btcusdt@trade");
        assert_eq!(BinanceAsset::ETH.stream_name(), "ethusdt@trade");
        assert_eq!(BinanceAsset::SOL.stream_name(), "solusdt@trade");
        assert_eq!(BinanceAsset::XRP.stream_name(), "xrpusdt@trade");
    }

    #[test]
    fn test_binance_asset_from_symbol() {
        assert_eq!(BinanceAsset::from_symbol("BTCUSDT"), Some(BinanceAsset::BTC));
        assert_eq!(BinanceAsset::from_symbol("btcusdt"), Some(BinanceAsset::BTC));
        assert_eq!(BinanceAsset::from_symbol("ETHUSDT"), Some(BinanceAsset::ETH));
        assert_eq!(BinanceAsset::from_symbol("SOLUSDT"), Some(BinanceAsset::SOL));
        assert_eq!(BinanceAsset::from_symbol("XRPUSDT"), Some(BinanceAsset::XRP));
        assert_eq!(BinanceAsset::from_symbol("UNKNOWN"), None);
        assert_eq!(BinanceAsset::from_symbol("BTC"), None);
    }

    #[test]
    fn test_binance_asset_symbol() {
        assert_eq!(BinanceAsset::BTC.symbol(), "BTC");
        assert_eq!(BinanceAsset::ETH.symbol(), "ETH");
        assert_eq!(BinanceAsset::SOL.symbol(), "SOL");
        assert_eq!(BinanceAsset::XRP.symbol(), "XRP");
    }

    #[test]
    fn test_binance_asset_all() {
        let all = BinanceAsset::all();
        assert_eq!(all.len(), 4);
        assert!(all.contains(&BinanceAsset::BTC));
        assert!(all.contains(&BinanceAsset::ETH));
        assert!(all.contains(&BinanceAsset::SOL));
        assert!(all.contains(&BinanceAsset::XRP));
    }

    #[test]
    fn test_parse_binance_stream_wrapper() {
        let json = r#"{
            "stream": "btcusdt@trade",
            "data": {
                "e": "trade",
                "E": 1766482935996,
                "s": "BTCUSDT",
                "t": 5697810014,
                "p": "87398.39000000",
                "q": "0.00103000",
                "T": 1766482935995,
                "m": false,
                "M": true
            }
        }"#;

        let wrapper: BinanceStreamWrapper = serde_json::from_str(json).unwrap();
        assert_eq!(wrapper.stream, "btcusdt@trade");
        assert_eq!(wrapper.data.event_type, "trade");
        assert_eq!(wrapper.data.symbol, "BTCUSDT");
        assert_eq!(wrapper.data.trade_id, 5697810014);
        assert_eq!(wrapper.data.price, "87398.39000000");
        assert_eq!(wrapper.data.quantity, "0.00103000");
        assert_eq!(wrapper.data.event_time, 1766482935996);
        assert_eq!(wrapper.data.trade_time, 1766482935995);
        assert!(!wrapper.data.is_buyer_maker);
        assert!(wrapper.data.ignore);
    }

    #[test]
    fn test_price_f64_parsing() {
        let json = r#"{
            "stream": "ethusdt@trade",
            "data": {
                "e": "trade",
                "E": 1000000,
                "s": "ETHUSDT",
                "t": 123,
                "p": "3456.78900000",
                "q": "1.5",
                "T": 1000000,
                "m": true
            }
        }"#;

        let wrapper: BinanceStreamWrapper = serde_json::from_str(json).unwrap();
        let price = wrapper.data.price_f64().unwrap();
        assert!((price - 3456.789).abs() < 0.0001);

        let qty = wrapper.data.quantity_f64().unwrap();
        assert!((qty - 1.5).abs() < 0.0001);
    }

    #[test]
    fn test_binance_route_equality() {
        let route1 = BinanceRoute::Trades;
        let route2 = BinanceRoute::Trades;
        assert_eq!(route1, route2);
    }
}
