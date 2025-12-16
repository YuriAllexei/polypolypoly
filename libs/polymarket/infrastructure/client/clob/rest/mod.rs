//! REST API client for Polymarket CLOB
//!
//! Split into focused modules:
//! - `orders`: Order placement methods
//! - `auth`: API key management
//! - `cancellation`: Order cancellation methods

mod auth;
mod cancellation;
mod orders;
mod queries;

use super::helpers::{parse_json, require_success};
use super::types::*;
use reqwest::Client;
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, warn};

#[derive(Error, Debug)]
pub enum RestError {
    #[error("HTTP request failed: {0}")]
    RequestFailed(#[from] reqwest::Error),

    #[error("API error: {0}")]
    ApiError(String),

    #[error("Authentication failed: {0}")]
    AuthFailed(#[from] super::super::auth::AuthError),

    #[error("Deserialization failed: {0}")]
    DeserializeFailed(String),
}

pub type Result<T> = std::result::Result<T, RestError>;

/// REST API client for Polymarket CLOB
pub struct RestClient {
    pub(crate) base_url: String,
    pub(crate) client: Client,
}

impl RestClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            base_url: base_url.into(),
            client,
        }
    }

    /// Get all simplified markets
    pub async fn get_markets(&self) -> Result<Vec<Market>> {
        let url = format!("{}/markets", self.base_url);

        debug!("Fetching markets from {}", url);

        let response = self.client.get(&url).send().await?;
        let response = require_success(response, "Failed to fetch markets").await?;

        let simplified: Vec<SimplifiedMarket> = parse_json(response).await?;

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
        let response = require_success(response, "Failed to fetch market").await?;

        let simplified: SimplifiedMarket = parse_json(response).await?;

        simplified
            .into_market()
            .map_err(|e| RestError::DeserializeFailed(e.to_string()))
    }

    /// Get orderbook for a specific token
    pub async fn get_orderbook(&self, token_id: &str) -> Result<OrderBook> {
        let url = format!("{}/book?token_id={}", self.base_url, token_id);

        debug!("Fetching orderbook for token {} from {}", token_id, url);

        let response = self.client.get(&url).send().await?;
        let response = require_success(response, "Failed to fetch orderbook").await?;

        parse_json(response).await
    }

    /// Get neg_risk status for a token (affects EIP-712 domain for signing)
    pub async fn get_neg_risk(&self, token_id: &str) -> Result<bool> {
        let url = format!("{}/neg-risk?token_id={}", self.base_url, token_id);

        debug!("Fetching neg_risk for token {}", token_id);

        let response = self
            .client
            .get(&url)
            .timeout(Duration::from_secs(5))
            .send()
            .await?;
        let response = require_success(response, "Failed to fetch neg_risk").await?;

        let neg_risk_resp: NegRiskResponse = parse_json(response).await?;
        Ok(neg_risk_resp.neg_risk)
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
