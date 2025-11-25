use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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

/// Price level in order book
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceLevel {
    pub price: String,  // String to avoid float precision issues
    pub size: String,   // String to avoid float precision issues
}

impl PriceLevel {
    pub fn price_f64(&self) -> f64 {
        self.price.parse().unwrap_or(0.0)
    }

    pub fn size_f64(&self) -> f64 {
        self.size.parse().unwrap_or(0.0)
    }
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
}

/// Order creation request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderArgs {
    pub token_id: String,
    pub price: f64,
    pub size: f64,
    pub side: Side,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub feeRateBps: Option<u64>,

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

/// Order response from API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderResponse {
    #[serde(rename = "orderID")]
    pub order_id: String,

    pub success: bool,

    #[serde(default)]
    pub error_msg: Option<String>,
}

/// User position
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub asset_id: String,
    pub market: String,
    pub size: String,

    #[serde(default)]
    pub side: Option<Side>,
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
