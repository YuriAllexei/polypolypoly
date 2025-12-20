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
use parking_lot::RwLock;
use reqwest::Client;
use std::error::Error as StdError;
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, info, warn};

/// Extract detailed error information from a reqwest error
fn describe_reqwest_error(err: &reqwest::Error) -> String {
    let mut details = Vec::new();

    if err.is_connect() {
        details.push("CONNECTION_FAILED");
    }
    if err.is_timeout() {
        details.push("TIMEOUT");
    }
    if err.is_request() {
        details.push("REQUEST_ERROR");
    }
    if err.is_builder() {
        details.push("BUILDER_ERROR");
    }
    if err.is_redirect() {
        details.push("REDIRECT_ERROR");
    }
    if err.is_body() {
        details.push("BODY_ERROR");
    }
    if err.is_decode() {
        details.push("DECODE_ERROR");
    }

    // Get the error source chain
    let mut source_chain = String::new();
    let mut source: Option<&(dyn StdError + 'static)> = StdError::source(err);
    while let Some(src) = source {
        if !source_chain.is_empty() {
            source_chain.push_str(" -> ");
        }
        source_chain.push_str(&src.to_string());
        source = src.source();
    }

    let type_info = if details.is_empty() {
        "UNKNOWN".to_string()
    } else {
        details.join("+")
    };

    if source_chain.is_empty() {
        format!("[{}] {}", type_info, err)
    } else {
        format!("[{}] {} (caused by: {})", type_info, err, source_chain)
    }
}

/// Build HTTP client matching official rs-clob-client exactly
/// The official client uses minimal settings with NO custom timeouts
fn build_http_client() -> Client {
    use reqwest::header;

    let mut headers = header::HeaderMap::new();
    // Match official rs-clob-client headers exactly
    headers.insert(
        header::USER_AGENT,
        header::HeaderValue::from_static("rs_clob_client"),
    );
    headers.insert(
        header::ACCEPT,
        header::HeaderValue::from_static("*/*"),
    );
    headers.insert(
        header::CONNECTION,
        header::HeaderValue::from_static("keep-alive"),
    );
    headers.insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/json"),
    );

    // Match official client: NO custom timeouts, use reqwest defaults
    Client::builder()
        .default_headers(headers)
        .build()
        .expect("Failed to build HTTP client")
}

#[derive(Error, Debug)]
pub enum RestError {
    #[error("HTTP request failed: {}", describe_reqwest_error(.0))]
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
///
/// Uses a persistent HTTP connection with auto-recreation on failure.
pub struct RestClient {
    pub(crate) base_url: String,
    client: RwLock<Client>,
}

impl RestClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            client: RwLock::new(build_http_client()),
        }
    }

    /// Get the HTTP client
    pub(crate) fn client(&self) -> Client {
        self.client.read().clone()
    }

    /// Recreate the HTTP client (forces new DNS resolution and connection)
    pub fn recreate_client(&self) {
        info!("[RestClient] Recreating HTTP client to force fresh connection");
        let new_client = build_http_client();
        *self.client.write() = new_client;
        info!("[RestClient] HTTP client recreated successfully");
    }

    /// Check HTTP connectivity to the CLOB API
    ///
    /// Makes a lightweight request to verify the network path is working.
    /// This helps detect connectivity issues early rather than at order time.
    pub async fn health_check(&self) -> Result<()> {
        let url = format!("{}/time", self.base_url);

        debug!("Checking CLOB API connectivity: {}", url);

        let response = self
            .client()
            .get(&url)
            .timeout(Duration::from_secs(5))
            .send()
            .await?;

        // We just care that we got a response, not what it says
        let status = response.status();
        debug!("CLOB API health check response: {}", status);

        Ok(())
    }

    /// Ensure connectivity before making a request
    /// If health check fails, recreate client and retry once
    pub async fn ensure_connectivity(&self) -> Result<()> {
        match self.health_check().await {
            Ok(_) => {
                debug!("[RestClient] Pre-flight connectivity check passed");
                Ok(())
            }
            Err(e) => {
                warn!("[RestClient] Pre-flight check failed: {}. Recreating client...", e);
                self.recreate_client();

                // Retry after recreation
                match self.health_check().await {
                    Ok(_) => {
                        info!("[RestClient] Connectivity restored after client recreation");
                        Ok(())
                    }
                    Err(e2) => {
                        warn!("[RestClient] Still failing after recreation: {}", e2);
                        Err(e2)
                    }
                }
            }
        }
    }

    /// Get all simplified markets
    pub async fn get_markets(&self) -> Result<Vec<Market>> {
        let url = format!("{}/markets", self.base_url);

        debug!("Fetching markets from {}", url);

        let response = self.client().get(&url).send().await?;
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

        let response = self.client().get(&url).send().await?;
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

        let response = self.client().get(&url).send().await?;
        let response = require_success(response, "Failed to fetch orderbook").await?;

        parse_json(response).await
    }

    /// Get neg_risk status for a token (affects EIP-712 domain for signing)
    pub async fn get_neg_risk(&self, token_id: &str) -> Result<bool> {
        let url = format!("{}/neg-risk?token_id={}", self.base_url, token_id);

        debug!("Fetching neg_risk for token {}", token_id);

        let response = self
            .client()
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
