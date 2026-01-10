use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// Re-export PriceLevel from domain
pub use crate::domain::orderbook::PriceLevel;

/// Represents a prediction market on Polymarket
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Market {
    #[serde(rename = "condition_id")]
    pub id: String,

    pub question: String,

    #[serde(rename = "end_date_iso")]
    pub resolution_time: DateTime<Utc>,

    #[serde(rename = "tokens")]
    pub outcomes: Vec<Outcome>,

    #[serde(default)]
    pub active: bool,

    #[serde(default)]
    pub closed: bool,
}

/// Represents an outcome/token in a market
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Outcome {
    #[serde(rename = "token_id")]
    pub id: String,

    pub outcome: String,

    #[serde(default)]
    pub price: Option<f64>,
}

/// Order book for a market
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBook {
    pub market: String,
    pub asset_id: String,
    pub bids: Vec<PriceLevel>,
    pub asks: Vec<PriceLevel>,

    #[serde(default)]
    pub timestamp: Option<String>,

    #[serde(default)]
    pub hash: Option<String>,

    #[serde(default)]
    pub min_order_size: Option<String>,

    #[serde(default)]
    pub tick_size: Option<String>,

    #[serde(default)]
    pub neg_risk: Option<bool>,
}

/// Order side
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum Side {
    Buy,
    Sell,
}

/// Order type
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum OrderType {
    GTC,  // Good Till Cancel
    FOK,  // Fill Or Kill
    GTD,  // Good Till Date
    FAK,  // Fill And Kill (partial fills allowed, rest cancelled)
}

impl OrderType {
    /// Convert to API string representation
    pub fn as_str(&self) -> &'static str {
        match self {
            OrderType::GTC => "GTC",
            OrderType::GTD => "GTD",
            OrderType::FOK => "FOK",
            OrderType::FAK => "FAK",
        }
    }
}

/// Order creation request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderArgs {
    pub token_id: String,
    pub price: f64,
    pub size: f64,
    pub side: Side,

    #[serde(rename = "feeRateBps", skip_serializing_if = "Option::is_none")]
    pub fee_rate_bps: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<u64>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiration: Option<u64>,
}

/// Market order request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketOrderArgs {
    pub token_id: String,
    pub amount: f64,  // In USD
    pub side: Side,
}

/// Signed order ready to submit
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignedOrder {
    #[serde(flatten)]
    pub order: OrderArgs,

    pub signature: String,

    #[serde(rename = "orderType")]
    pub order_type: OrderType,
}

/// Order response from API (legacy)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderResponse {
    #[serde(rename = "orderID")]
    pub order_id: String,

    pub success: bool,

    #[serde(default)]
    pub error_msg: Option<String>,
}

/// Order placement response from CLOB API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderPlacementResponse {
    /// Order ID if placement was successful
    #[serde(rename = "orderID", default)]
    pub order_id: Option<String>,

    /// Whether the order was successfully placed
    pub success: bool,

    /// Error message if placement failed
    #[serde(rename = "errorMsg", default)]
    pub error_msg: Option<String>,

    /// Order status: "matched", "live", "delayed", "unmatched"
    #[serde(default)]
    pub status: Option<String>,

    /// Transaction hashes if order was matched
    #[serde(rename = "orderHashes", default)]
    pub order_hashes: Option<Vec<String>>,
}

/// Batch order placement response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchOrderResponse {
    /// Individual order responses
    #[serde(default)]
    pub orders: Vec<OrderPlacementResponse>,

    /// Overall success status
    pub success: bool,
}

/// Response from order cancellation endpoints
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelResponse {
    pub canceled: Vec<String>,
    #[serde(default)]
    pub not_canceled: HashMap<String, String>,
}

/// Nonce response from exchange
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NonceResponse {
    /// Current nonce value as string
    pub nonce: String,
}

/// Neg risk response from exchange
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NegRiskResponse {
    /// Whether the token uses neg_risk exchange
    pub neg_risk: bool,
}

/// User position from Data API
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    /// Token ID (called "asset" in Data API)
    #[serde(alias = "asset")]
    pub asset_id: String,

    /// Condition ID (market)
    #[serde(default)]
    pub condition_id: Option<String>,

    /// Position size (number of shares) - stored as string for flexibility
    #[serde(deserialize_with = "deserialize_size")]
    pub size: String,

    /// Average entry price
    #[serde(default)]
    pub avg_price: Option<f64>,

    /// Current price
    #[serde(default)]
    pub cur_price: Option<f64>,

    /// Realized P&L
    #[serde(default)]
    pub realized_pnl: Option<f64>,

    /// Outcome name (YES/NO)
    #[serde(default)]
    pub outcome: Option<String>,

    /// Whether position is mergeable
    #[serde(default)]
    pub mergeable: Option<bool>,

    /// Opposite asset (for merge)
    #[serde(default)]
    pub opposite_asset: Option<String>,

    #[serde(default)]
    pub side: Option<Side>,
}

/// Deserialize size - accepts either string or number from API, returns String
fn deserialize_size<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    // Try to deserialize as f64 first (most common from Data API)
    // Fall back to string if that fails
    let value: serde_json::Value = serde::Deserialize::deserialize(deserializer)?;

    match value {
        serde_json::Value::Number(n) => Ok(n.to_string()),
        serde_json::Value::String(s) => Ok(s),
        _ => Err(serde::de::Error::custom("expected string or number for size")),
    }
}

/// API credentials (L2 auth)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiCredentials {
    pub key: String,
    pub secret: String,
    pub passphrase: String,
}

/// Simplified market response from API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimplifiedMarket {
    #[serde(rename = "condition_id")]
    pub condition_id: String,

    pub question: String,

    #[serde(rename = "end_date_iso")]
    pub end_date_iso: String,

    #[serde(default)]
    pub active: bool,

    #[serde(default)]
    pub closed: bool,

    #[serde(default)]
    pub tokens: Vec<SimplifiedToken>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimplifiedToken {
    #[serde(rename = "token_id")]
    pub token_id: String,

    pub outcome: String,

    #[serde(default)]
    pub price: Option<String>,
}

impl SimplifiedMarket {
    /// Convert to Market struct
    pub fn into_market(self) -> Result<Market, chrono::ParseError> {
        Ok(Market {
            id: self.condition_id,
            question: self.question,
            resolution_time: DateTime::parse_from_rfc3339(&self.end_date_iso)?
                .with_timezone(&Utc),
            outcomes: self
                .tokens
                .into_iter()
                .map(|t| Outcome {
                    id: t.token_id,
                    outcome: t.outcome,
                    price: t.price.and_then(|p| p.parse().ok()),
                })
                .collect(),
            active: self.active,
            closed: self.closed,
        })
    }
}

/// Asset type for balance queries
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "UPPERCASE")]
pub enum AssetType {
    Collateral,
    Conditional,
}

/// Query parameters for fetching open orders
#[derive(Debug, Clone, Default)]
pub struct OpenOrderParams {
    pub id: Option<String>,
    pub market: Option<String>,
    pub asset_id: Option<String>,
}

/// Query parameters for fetching trades
#[derive(Debug, Clone, Default)]
pub struct TradeParams {
    pub id: Option<String>,
    pub maker_address: Option<String>,
    pub market: Option<String>,
    pub asset_id: Option<String>,
    pub before: Option<i64>,
    pub after: Option<i64>,
}

/// Query parameters for balance/allowance
#[derive(Debug, Clone, Default)]
pub struct BalanceAllowanceParams {
    pub asset_type: Option<AssetType>,
    pub token_id: Option<String>,
    pub signature_type: Option<u8>,
}

/// Balance and allowance response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BalanceAllowance {
    #[serde(default)]
    pub balance: String,
    #[serde(default)]
    pub allowance: String,
}

/// Paginated response wrapper
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginatedResponse<T> {
    #[serde(default)]
    pub data: Vec<T>,
    pub next_cursor: String,
}

/// Open order from API (flexible type)
pub type OpenOrder = serde_json::Value;

/// Trade from API (flexible type)
pub type Trade = serde_json::Value;

/// Best price opportunity in orderbook
#[derive(Debug, Clone)]
pub struct BestOpportunity {
    pub side: Side,
    pub price: f64,
    pub size: f64,
    pub token_id: String,
}

impl OrderBook {
    /// Find the best opportunity (highest probability side)
    pub fn best_opportunity(&self) -> Option<BestOpportunity> {
        let best_bid = self.bids.first().map(|b| b.price_f64()).unwrap_or(0.0);
        let best_ask = self.asks.first().map(|a| a.price_f64()).unwrap_or(1.0);

        // Higher price = higher probability
        if best_bid >= best_ask {
            // Buy side is better (or equal)
            Some(BestOpportunity {
                side: Side::Buy,
                price: best_bid,
                size: self.bids.first()?.size_f64(),
                token_id: self.asset_id.clone(),
            })
        } else {
            // Sell side is better (inverse probability: 1 - ask_price)
            let sell_probability = 1.0 - best_ask;
            let buy_probability = best_bid;

            if sell_probability > buy_probability {
                Some(BestOpportunity {
                    side: Side::Sell,
                    price: 1.0 - best_ask,  // Convert to probability
                    size: self.asks.first()?.size_f64(),
                    token_id: self.asset_id.clone(),
                })
            } else {
                Some(BestOpportunity {
                    side: Side::Buy,
                    price: best_bid,
                    size: self.bids.first()?.size_f64(),
                    token_id: self.asset_id.clone(),
                })
            }
        }
    }

    /// Get the best ask (lowest/cheapest price to buy at)
    pub fn best_ask(&self) -> Option<&PriceLevel> {
        self.asks.last()
    }

    /// Calculate total cost to sweep entire ask side of orderbook
    /// Returns the sum of (price * size) for all asks
    pub fn total_ask_sweep_cost(&self) -> f64 {
        self.asks.iter()
            .map(|ask| ask.price_f64() * ask.size_f64())
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_data_api_position_response() {
        // Test with full API response format
        let json = r#"[{"proxyWallet":"0x79bf6aa04182fae6098f5cde298051cb73d8cb4f","asset":"49676780810689444200318417088291763726636300097901733086536799145920190033679","conditionId":"0x2fc5c2a7fbe700ce96e945a22a27536c633a1fa0ca396b4bee37611d3eedff18","size":104.886789,"avgPrice":0.894118,"initialValue":93.781166007102,"currentValue":90.727072485,"cashPnl":-3.0540935221019985,"percentPnl":-3.256617135545866,"totalBought":156,"realizedPnl":-13.388357,"percentRealizedPnl":-34.95447892135401,"curPrice":0.865,"redeemable":false,"mergeable":true,"title":"Bitcoin Up or Down","outcome":"Down","outcomeIndex":1,"oppositeOutcome":"Up","oppositeAsset":"74500928740699287248693336296486952739964042785657855592532954908753339798970","endDate":"2026-01-08","negativeRisk":false}]"#;

        let result: Result<Vec<Position>, _> = serde_json::from_str(json);
        println!("Parse result: {:?}", result);

        let positions = result.expect("Failed to parse positions");
        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0].asset_id, "49676780810689444200318417088291763726636300097901733086536799145920190033679");
        assert_eq!(positions[0].size, "104.886789");
        assert_eq!(positions[0].avg_price, Some(0.894118));
        assert_eq!(positions[0].mergeable, Some(true));
        assert_eq!(positions[0].outcome, Some("Down".to_string()));
    }

    #[test]
    fn test_parse_open_order_response_for_reconciliation() {
        // Test OpenOrder parsing and ID extraction for reconciliation
        // OpenOrder is serde_json::Value, so we just need to verify ID extraction works
        let json = r#"[
            {
                "id": "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
                "status": "LIVE",
                "owner": "0xowner",
                "maker_address": "0xmaker",
                "market": "0xmarket123",
                "asset_id": "12345678901234567890",
                "side": "BUY",
                "original_size": "100.5",
                "size_matched": "0",
                "price": "0.55"
            },
            {
                "id": "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890",
                "status": "LIVE",
                "owner": "0xowner",
                "maker_address": "0xmaker",
                "market": "0xmarket123",
                "asset_id": "12345678901234567890",
                "side": "SELL",
                "original_size": "50",
                "size_matched": "25",
                "price": "0.60"
            }
        ]"#;

        let orders: Vec<OpenOrder> = serde_json::from_str(json).expect("Failed to parse orders");
        assert_eq!(orders.len(), 2);

        // Verify ID extraction works (same logic as reconcile_orders)
        let order_ids: Vec<String> = orders
            .iter()
            .filter_map(|o| o.get("id").and_then(|v| v.as_str()).map(String::from))
            .collect();

        assert_eq!(order_ids.len(), 2);
        assert_eq!(order_ids[0], "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef");
        assert_eq!(order_ids[1], "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890");

        // Verify other fields are accessible
        assert_eq!(orders[0].get("side").and_then(|v| v.as_str()), Some("BUY"));
        assert_eq!(orders[0].get("original_size").and_then(|v| v.as_str()), Some("100.5"));
        assert_eq!(orders[1].get("size_matched").and_then(|v| v.as_str()), Some("25"));
    }

    #[test]
    fn test_parse_open_order_with_numeric_fields() {
        // Test that numeric fields (if API ever returns them) are handled
        // Even if original_size comes as number, we don't parse it in reconciliation
        let json = r#"[{
            "id": "0x123abc",
            "status": "LIVE",
            "original_size": 100.5,
            "price": 0.55
        }]"#;

        let orders: Vec<OpenOrder> = serde_json::from_str(json).expect("Failed to parse orders");
        assert_eq!(orders.len(), 1);

        // ID extraction still works
        let id = orders[0].get("id").and_then(|v| v.as_str());
        assert_eq!(id, Some("0x123abc"));

        // Numeric original_size won't break anything - we just don't use it for reconciliation
        // (reconciliation only cares about the "id" field)
        let size = orders[0].get("original_size");
        assert!(size.is_some());
        assert!(size.unwrap().is_f64()); // It's a number, but that's OK
    }
}
