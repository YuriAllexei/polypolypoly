use super::types::{Event, GammaFilters, Market};
use chrono::{DateTime, Utc};
use reqwest::Client;
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, info, warn};

#[derive(Error, Debug)]
pub enum GammaError {
    #[error("HTTP request failed: {0}")]
    RequestFailed(#[from] reqwest::Error),

    #[error("API error: {0}")]
    ApiError(String),

    #[error("Deserialization failed: {0}")]
    DeserializeFailed(String),

    #[error("Rate limit exceeded")]
    RateLimitExceeded,
}

pub type Result<T> = std::result::Result<T, GammaError>;

/// Gamma Markets API client
pub struct GammaClient {
    base_url: String,
    client: Client,
}

impl GammaClient {
    /// Create new Gamma API client
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            client: Client::new(),
        }
    }

    /// Fetch ALL active markets with pagination
    pub async fn get_all_active_markets(&self) -> Result<Vec<Market>> {
        let filters = GammaFilters {
            active: Some(true),
            closed: Some(false),
            archived: Some(false),
            ..Default::default()
        };

        self.get_all_markets_with_filters(filters).await
    }

    /// Fetch ALL markets with custom filters and pagination
    pub async fn get_all_markets_with_filters(
        &self,
        filters: GammaFilters,
    ) -> Result<Vec<Market>> {
        let mut all_markets = Vec::new();
        let mut offset = 0;
        const LIMIT: usize = 100; // Max per Gamma API spec

        info!("Starting paginated market fetch");

        loop {
            debug!("Fetching page: offset={}, limit={}", offset, LIMIT);

            let markets = self.get_markets_page(LIMIT, offset, filters.clone()).await?;

            let count = markets.len();
            debug!("Fetched {} markets in this page", count);

            all_markets.extend(markets);

            // If we got fewer than limit, we've reached the end
            if count < LIMIT {
                debug!("Reached end of pagination (got {} < {})", count, LIMIT);
                break;
            }

            offset += LIMIT;

            // Rate limit protection: 100 req/10s = ~100ms between requests
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        info!("Fetched total of {} markets", all_markets.len());
        Ok(all_markets)
    }

    /// Fetch single page of markets
    pub async fn get_markets_page(
        &self,
        limit: usize,
        offset: usize,
        filters: GammaFilters,
    ) -> Result<Vec<Market>> {
        let url = format!("{}/markets", self.base_url);

        let mut params = filters.to_query_params();
        params.push(("limit".to_string(), limit.to_string()));
        params.push(("offset".to_string(), offset.to_string()));
        params.push(("order".to_string(), "id".to_string()));
        params.push(("ascending".to_string(), "false".to_string()));

        debug!("GET {} with {} params", url, params.len());

        let response = self.client.get(&url).query(&params).send().await?;

        let status = response.status();

        if status == 429 {
            warn!("Rate limit exceeded");
            return Err(GammaError::RateLimitExceeded);
        }

        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(GammaError::ApiError(format!(
                "Failed to fetch markets ({}): {}",
                status, error_text
            )));
        }

        let markets: Vec<Market> = response
            .json()
            .await
            .map_err(|e| GammaError::DeserializeFailed(e.to_string()))?;

        Ok(markets)
    }

    /// Get new markets since timestamp
    pub async fn get_new_markets(&self, since: DateTime<Utc>) -> Result<Vec<Market>> {
        let filters = GammaFilters {
            active: Some(true),
            closed: Some(false),
            start_date_min: Some(since),
            ..Default::default()
        };

        // New markets are typically few, so just fetch first page
        self.get_markets_page(100, 0, filters).await
    }

    /// Fetch ALL active events (preferred method - more efficient)
    pub async fn get_all_active_events(&self) -> Result<Vec<Event>> {
        let mut all_events = Vec::new();
        let mut offset = 0;
        const LIMIT: usize = 100;

        info!("Starting paginated event fetch");

        loop {
            debug!("Fetching events page: offset={}, limit={}", offset, LIMIT);

            let events = self.get_events_page(LIMIT, offset).await?;

            let count = events.len();
            debug!("Fetched {} events in this page", count);

            all_events.extend(events);

            if count < LIMIT {
                debug!("Reached end of pagination");
                break;
            }

            offset += LIMIT;

            // Rate limit protection
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        info!("Fetched total of {} events", all_events.len());
        Ok(all_events)
    }

    /// Fetch single page of events
    pub async fn get_events_page(&self, limit: usize, offset: usize) -> Result<Vec<Event>> {
        let url = format!("{}/events", self.base_url);

        // Store strings before borrowing
        let limit_str = limit.to_string();
        let offset_str = offset.to_string();

        let params = vec![
            ("closed", "false"),
            ("limit", &limit_str),
            ("offset", &offset_str),
            ("order", "id"),
            ("ascending", "false"),
        ];

        debug!("GET {} with params {:?}", url, params);

        let response = self.client.get(&url).query(&params).send().await?;

        let status = response.status();

        if status == 429 {
            warn!("Rate limit exceeded");
            return Err(GammaError::RateLimitExceeded);
        }

        if !status.is_success() {
            let error_text = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(GammaError::ApiError(format!(
                "Failed to fetch events ({}): {}",
                status, error_text
            )));
        }

        let events: Vec<Event> = response
            .json()
            .await
            .map_err(|e| GammaError::DeserializeFailed(e.to_string()))?;

        Ok(events)
    }

    /// Extract all markets from events
    pub fn extract_markets_from_events(events: Vec<Event>) -> Vec<Market> {
        events
            .into_iter()
            .filter_map(|event| event.markets)
            .flatten()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gamma_client_creation() {
        let client = GammaClient::new("https://gamma-api.polymarket.com");
        assert_eq!(client.base_url, "https://gamma-api.polymarket.com");
    }

    #[test]
    fn test_filters_to_query_params() {
        let filters = GammaFilters {
            active: Some(true),
            closed: Some(false),
            ..Default::default()
        };

        let params = filters.to_query_params();
        assert!(params.iter().any(|(k, v)| k == "active" && v == "true"));
        assert!(params.iter().any(|(k, v)| k == "closed" && v == "false"));
    }
}
