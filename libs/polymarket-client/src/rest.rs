use crate::auth::PolymarketAuth;
use crate::types::*;
use reqwest::Client;
use serde_json::json;
use thiserror::Error;
use tracing::{debug, warn};

#[derive(Error, Debug)]
pub enum RestError {
    #[error("HTTP request failed: {0}")]
    RequestFailed(#[from] reqwest::Error),

    #[error("API error: {0}")]
    ApiError(String),

    #[error("Authentication failed: {0}")]
    AuthFailed(#[from] crate::auth::AuthError),

    #[error("Deserialization failed: {0}")]
    DeserializeFailed(String),
}

pub type Result<T> = std::result::Result<T, RestError>;

/// REST API client for Polymarket CLOB
pub struct RestClient {
    base_url: String,
    client: Client,
}

impl RestClient {
    /// Create new REST client
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            client: Client::new(),
        }
    }

    /// Get all simplified markets
    pub async fn get_markets(&self) -> Result<Vec<Market>> {
        let url = format!("{}/markets", self.base_url);

        debug!("Fetching markets from {}", url);

        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(RestError::ApiError(format!(
                "Failed to fetch markets: {}",
                response.status()
            )));
        }

        let simplified: Vec<SimplifiedMarket> = response
            .json()
            .await
            .map_err(|e| RestError::DeserializeFailed(e.to_string()))?;

        // Convert to Market structs
        let mut markets = Vec::new();
        for sm in simplified {
            match sm.into_market() {
                Ok(market) => markets.push(market),
                Err(e) => {
                    warn!("Failed to parse market: {}", e);
                    continue;
                }
            }
        }

        debug!("Fetched {} markets", markets.len());
        Ok(markets)
    }

    /// Get specific market by condition ID
    pub async fn get_market(&self, condition_id: &str) -> Result<Market> {
        let url = format!("{}/markets/{}", self.base_url, condition_id);

        debug!("Fetching market {} from {}", condition_id, url);

        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(RestError::ApiError(format!(
                "Failed to fetch market: {}",
                response.status()
            )));
        }

        let simplified: SimplifiedMarket = response
            .json()
            .await
            .map_err(|e| RestError::DeserializeFailed(e.to_string()))?;

        simplified
            .into_market()
            .map_err(|e| RestError::DeserializeFailed(e.to_string()))
    }

    /// Get orderbook for a specific token
    pub async fn get_orderbook(&self, token_id: &str) -> Result<OrderBook> {
        let url = format!("{}/book?token_id={}", self.base_url, token_id);

        debug!("Fetching orderbook for token {} from {}", token_id, url);

        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(RestError::ApiError(format!(
                "Failed to fetch orderbook: {}",
                response.status()
            )));
        }

        response
            .json()
            .await
            .map_err(|e| RestError::DeserializeFailed(e.to_string()))
    }

    /// Create or derive API key credentials (L2 auth)
    pub async fn create_api_key(&self, auth: &PolymarketAuth) -> Result<ApiCredentials> {
        let url = format!("{}/auth/api-key", self.base_url);
        let timestamp = PolymarketAuth::current_timestamp();

        debug!("Creating API key");

        // Get L1 headers
        let headers = auth.l1_headers(timestamp, 0).await?;

        // Build request
        let mut req = self.client.post(&url);
        for (key, value) in headers {
            req = req.header(key, value);
        }

        let response = req.send().await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            return Err(RestError::ApiError(format!(
                "Failed to create API key: {}",
                error_text
            )));
        }

        response
            .json()
            .await
            .map_err(|e| RestError::DeserializeFailed(e.to_string()))
    }

    /// Get API credentials (either derive from existing or create new)
    pub async fn get_or_create_api_creds(&self, auth: &PolymarketAuth) -> Result<ApiCredentials> {
        // Try to derive first (deterministic)
        match self.derive_api_key(auth).await {
            Ok(creds) => Ok(creds),
            Err(_) => {
                // If derivation fails, create new
                self.create_api_key(auth).await
            }
        }
    }

    /// Derive API key (deterministic from private key)
    pub async fn derive_api_key(&self, auth: &PolymarketAuth) -> Result<ApiCredentials> {
        let url = format!("{}/auth/derive-api-key", self.base_url);
        let timestamp = PolymarketAuth::current_timestamp();

        debug!("Deriving API key");

        // Get L1 headers
        let headers = auth.l1_headers(timestamp, 0).await?;

        // Build request
        let mut req = self.client.get(&url);
        for (key, value) in headers {
            req = req.header(key, value);
        }

        let response = req.send().await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            return Err(RestError::ApiError(format!(
                "Failed to derive API key: {}",
                error_text
            )));
        }

        response
            .json()
            .await
            .map_err(|e| RestError::DeserializeFailed(e.to_string()))
    }

    /// Place a limit order
    pub async fn place_order(
        &self,
        auth: &PolymarketAuth,
        order_args: &OrderArgs,
        order_type: OrderType,
    ) -> Result<OrderResponse> {
        let url = format!("{}/order", self.base_url);
        let timestamp = PolymarketAuth::current_timestamp();

        debug!("Placing {:?} order for token {}", order_type, order_args.token_id);

        // Serialize order body
        let body_json = json!({
            "order": order_args,
            "orderType": order_type,
        });
        let body = serde_json::to_string(&body_json)
            .map_err(|e| RestError::ApiError(e.to_string()))?;

        // Get L2 headers
        let headers = auth.l2_headers(timestamp, "POST", "/order", &body)?;

        // Build request
        let mut req = self.client.post(&url).header("Content-Type", "application/json");
        for (key, value) in headers {
            req = req.header(key, value);
        }

        let response = req.body(body).send().await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            return Err(RestError::ApiError(format!(
                "Failed to place order: {}",
                error_text
            )));
        }

        response
            .json()
            .await
            .map_err(|e| RestError::DeserializeFailed(e.to_string()))
    }

    /// Place a market order (buy/sell by amount)
    pub async fn place_market_order(
        &self,
        auth: &PolymarketAuth,
        market_order: &MarketOrderArgs,
        order_type: OrderType,
    ) -> Result<OrderResponse> {
        debug!(
            "Placing market {:?} order for {} USD",
            market_order.side, market_order.amount
        );

        // Get current best price for the side
        let orderbook = self.get_orderbook(&market_order.token_id).await?;

        // Calculate price and size
        let (price, size) = match market_order.side {
            Side::Buy => {
                // For market buy, use best ask price
                let best_ask = orderbook
                    .asks
                    .first()
                    .ok_or_else(|| RestError::ApiError("No asks available".to_string()))?;
                let price = best_ask.price_f64();
                let size = market_order.amount / price;
                (price, size)
            }
            Side::Sell => {
                // For market sell, use best bid price
                let best_bid = orderbook
                    .bids
                    .first()
                    .ok_or_else(|| RestError::ApiError("No bids available".to_string()))?;
                let price = best_bid.price_f64();
                let size = market_order.amount / price;
                (price, size)
            }
        };

        // Create limit order with marketable price
        let order_args = OrderArgs {
            token_id: market_order.token_id.clone(),
            price,
            size,
            side: market_order.side,
            feeRateBps: None,
            nonce: None,
            expiration: None,
        };

        self.place_order(auth, &order_args, order_type).await
    }

    /// Get user positions
    pub async fn get_positions(&self, auth: &PolymarketAuth) -> Result<Vec<Position>> {
        let url = format!("{}/positions", self.base_url);
        let timestamp = PolymarketAuth::current_timestamp();

        debug!("Fetching user positions");

        // Get L2 headers
        let headers = auth.l2_headers(timestamp, "GET", "/positions", "")?;

        // Build request
        let mut req = self.client.get(&url);
        for (key, value) in headers {
            req = req.header(key, value);
        }

        let response = req.send().await?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
            return Err(RestError::ApiError(format!(
                "Failed to fetch positions: {}",
                error_text
            )));
        }

        response
            .json()
            .await
            .map_err(|e| RestError::DeserializeFailed(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = RestClient::new("https://clob.polymarket.com");
        assert_eq!(client.base_url, "https://clob.polymarket.com");
    }
}
