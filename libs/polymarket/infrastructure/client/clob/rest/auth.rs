//! API key management methods for RestClient

use ethers::types::Address;
use super::super::super::auth::PolymarketAuth;
use super::super::helpers::{extract_api_error, parse_json, with_headers};
use super::super::types::*;
use super::{RestClient, Result, RestError};
use tracing::debug;

impl RestClient {
    /// Create or derive API key credentials (L2 auth)
    pub async fn create_api_key(&self, auth: &PolymarketAuth) -> Result<ApiCredentials> {
        let url = format!("{}/auth/api-key", self.base_url);
        let timestamp = PolymarketAuth::current_timestamp();

        debug!("Creating API key");

        let headers = auth.l1_headers(timestamp, 0).await?;
        let req = with_headers(self.client().post(&url), headers);
        let response = req.send().await?;

        if !response.status().is_success() {
            return Err(extract_api_error(response, "Failed to create API key").await);
        }

        parse_json(response).await
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

        let headers = auth.l1_headers(timestamp, 0).await?;
        let req = with_headers(self.client().get(&url), headers);
        let response = req.send().await?;

        if !response.status().is_success() {
            return Err(extract_api_error(response, "Failed to derive API key").await);
        }

        parse_json(response).await
    }

    /// Get maker's current nonce from the exchange
    pub async fn get_nonce(&self, auth: &PolymarketAuth) -> Result<u64> {
        let maker = format!("{:?}", auth.address());
        let path = format!("/nonce?maker={}", maker);
        let url = format!("{}{}", self.base_url, path);
        let timestamp = PolymarketAuth::current_timestamp();

        debug!("Fetching nonce for maker {}", maker);

        let headers = auth.l2_headers(timestamp, "GET", &path, "")?;
        let req = with_headers(self.client().get(&url), headers);
        let response = req.send().await?;

        if !response.status().is_success() {
            return Err(extract_api_error(response, "Failed to fetch nonce").await);
        }

        let nonce_resp: NonceResponse = parse_json(response).await?;
        nonce_resp
            .nonce
            .parse()
            .map_err(|e| RestError::ApiError(format!("Failed to parse nonce: {}", e)))
    }

    /// Get user positions from the Data API
    /// Note: This uses the Data API (data-api.polymarket.com), not the CLOB API
    /// Use maker_address (proxy wallet) not signer address - positions are held by proxy
    pub async fn get_positions(&self, address: Address) -> Result<Vec<Position>> {
        // Format address as lowercase hex with 0x prefix
        let address_str = format!("{:?}", address);
        let url = format!(
            "https://data-api.polymarket.com/positions?user={}",
            address_str
        );

        debug!("Fetching positions from: {}", url);

        // Data API doesn't require authentication - just GET with address
        let response = self.client().get(&url).send().await?;

        if !response.status().is_success() {
            return Err(extract_api_error(response, "Failed to fetch positions").await);
        }

        parse_json(response).await
    }
}
