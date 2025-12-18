//! Data API client for Polymarket
//!
//! Provides access to user positions and related data from the Data API.

use super::types::{Position, PositionFilters};
use reqwest::Client;
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, info, warn};

/// Default Data API base URL
const DATA_API_BASE_URL: &str = "https://data-api.polymarket.com";

/// Default limit for pagination
const DEFAULT_LIMIT: u32 = 100;

/// Maximum limit per API spec
const MAX_LIMIT: u32 = 500;

/// Maximum offset per API spec
const MAX_OFFSET: u32 = 10000;

#[derive(Error, Debug)]
pub enum DataApiError {
    #[error("HTTP request failed: {0}")]
    RequestFailed(#[from] reqwest::Error),

    #[error("API error: {0}")]
    ApiError(String),

    #[error("Deserialization failed: {0}")]
    DeserializeFailed(String),

    #[error("Rate limit exceeded")]
    RateLimitExceeded,

    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),
}

pub type Result<T> = std::result::Result<T, DataApiError>;

/// Data API client for fetching user positions
pub struct DataApiClient {
    base_url: String,
    client: Client,
}

impl DataApiClient {
    /// Create new Data API client with default base URL
    pub fn new() -> Self {
        Self::with_base_url(DATA_API_BASE_URL)
    }

    /// Create new Data API client with custom base URL
    pub fn with_base_url(base_url: impl Into<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(10))
            .pool_idle_timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(5)
            .tcp_keepalive(Duration::from_secs(15))
            .build()
            .expect("Failed to build HTTP client");

        Self {
            base_url: base_url.into(),
            client,
        }
    }

    /// Get positions for a user address (single page)
    ///
    /// # Arguments
    /// * `user` - Wallet address (0x-prefixed)
    /// * `filters` - Optional query filters
    ///
    /// # Returns
    /// Vector of positions for the user
    pub async fn get_positions(
        &self,
        user: &str,
        filters: Option<PositionFilters>,
    ) -> Result<Vec<Position>> {
        let url = format!("{}/positions", self.base_url);

        // Build query parameters
        let mut params = vec![("user".to_string(), user.to_string())];

        if let Some(f) = filters {
            params.extend(f.to_query_params());
        }

        debug!("GET {} with {} params", url, params.len());

        let response = self.client.get(&url).query(&params).send().await?;

        let status = response.status();

        if status == 429 {
            warn!("Rate limit exceeded on Data API");
            return Err(DataApiError::RateLimitExceeded);
        }

        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(DataApiError::ApiError(format!(
                "Failed to fetch positions ({}): {}",
                status, error_text
            )));
        }

        let positions: Vec<Position> = response
            .json()
            .await
            .map_err(|e| DataApiError::DeserializeFailed(e.to_string()))?;

        debug!("Fetched {} positions for user {}", positions.len(), user);
        Ok(positions)
    }

    /// Get all positions for a user with automatic pagination
    ///
    /// Fetches all pages of positions up to the API's maximum offset limit.
    ///
    /// # Arguments
    /// * `user` - Wallet address (0x-prefixed)
    /// * `filters` - Optional query filters (limit/offset will be managed automatically)
    ///
    /// # Returns
    /// Vector of all positions for the user
    pub async fn get_all_positions(
        &self,
        user: &str,
        filters: Option<PositionFilters>,
    ) -> Result<Vec<Position>> {
        let mut all_positions = Vec::new();
        let mut offset: u32 = 0;
        let limit = DEFAULT_LIMIT;

        // Clone base filters to modify for pagination
        let base_filters = filters.unwrap_or_default();

        info!("Starting paginated position fetch for user {}", user);

        loop {
            // Check offset limit
            if offset >= MAX_OFFSET {
                warn!(
                    "Reached maximum offset ({}) while fetching positions",
                    MAX_OFFSET
                );
                break;
            }

            debug!("Fetching positions page: offset={}, limit={}", offset, limit);

            // Build filters for this page
            let page_filters = PositionFilters {
                limit: Some(limit),
                offset: Some(offset),
                market: base_filters.market.clone(),
                event_id: base_filters.event_id.clone(),
                size_threshold: base_filters.size_threshold,
                redeemable: base_filters.redeemable,
                mergeable: base_filters.mergeable,
                sort_by: base_filters.sort_by,
                sort_direction: base_filters.sort_direction,
                title: base_filters.title.clone(),
            };

            let positions = self.get_positions(user, Some(page_filters)).await?;

            let count = positions.len();
            debug!("Fetched {} positions in this page", count);

            all_positions.extend(positions);

            // If we got fewer than limit, we've reached the end
            if count < limit as usize {
                debug!("Reached end of pagination (got {} < {})", count, limit);
                break;
            }

            offset += limit;

            // Rate limit protection: small delay between requests
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        info!(
            "Fetched total of {} positions for user {}",
            all_positions.len(),
            user
        );
        Ok(all_positions)
    }

    /// Get positions for specific market(s)
    ///
    /// Convenience method to fetch positions filtered by condition IDs.
    ///
    /// # Arguments
    /// * `user` - Wallet address (0x-prefixed)
    /// * `condition_ids` - Market condition IDs to filter by
    ///
    /// # Returns
    /// Vector of positions in the specified markets
    pub async fn get_positions_for_market(
        &self,
        user: &str,
        condition_ids: &[String],
    ) -> Result<Vec<Position>> {
        if condition_ids.is_empty() {
            return Err(DataApiError::InvalidParameter(
                "At least one condition ID is required".to_string(),
            ));
        }

        let filters = PositionFilters {
            market: Some(condition_ids.to_vec()),
            ..Default::default()
        };

        self.get_positions(user, Some(filters)).await
    }

    /// Get positions for a specific event
    ///
    /// # Arguments
    /// * `user` - Wallet address (0x-prefixed)
    /// * `event_ids` - Event IDs to filter by
    ///
    /// # Returns
    /// Vector of positions in the specified events
    pub async fn get_positions_for_event(
        &self,
        user: &str,
        event_ids: &[i64],
    ) -> Result<Vec<Position>> {
        if event_ids.is_empty() {
            return Err(DataApiError::InvalidParameter(
                "At least one event ID is required".to_string(),
            ));
        }

        let filters = PositionFilters {
            event_id: Some(event_ids.to_vec()),
            ..Default::default()
        };

        self.get_positions(user, Some(filters)).await
    }

    /// Get redeemable positions (market resolved, can claim winnings)
    ///
    /// # Arguments
    /// * `user` - Wallet address (0x-prefixed)
    ///
    /// # Returns
    /// Vector of redeemable positions
    pub async fn get_redeemable_positions(&self, user: &str) -> Result<Vec<Position>> {
        let filters = PositionFilters {
            redeemable: Some(true),
            ..Default::default()
        };

        self.get_all_positions(user, Some(filters)).await
    }

    /// Health check for the Data API
    pub async fn health_check(&self) -> Result<()> {
        let url = format!("{}/", self.base_url);

        debug!("Checking Data API connectivity: {}", url);

        let response = self
            .client
            .get(&url)
            .timeout(Duration::from_secs(5))
            .send()
            .await?;

        let status = response.status();
        debug!("Data API health check response: {}", status);

        if !status.is_success() {
            return Err(DataApiError::ApiError(format!(
                "Health check failed: {}",
                status
            )));
        }

        Ok(())
    }
}

impl Default for DataApiClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_creation() {
        let client = DataApiClient::new();
        assert_eq!(client.base_url, DATA_API_BASE_URL);
    }

    #[test]
    fn test_client_with_custom_url() {
        let client = DataApiClient::with_base_url("https://custom.api.com");
        assert_eq!(client.base_url, "https://custom.api.com");
    }

    #[test]
    fn test_default_trait() {
        let client = DataApiClient::default();
        assert_eq!(client.base_url, DATA_API_BASE_URL);
    }
}
