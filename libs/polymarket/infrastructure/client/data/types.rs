//! Data API types for positions and related entities

use serde::{Deserialize, Serialize};

// =============================================================================
// Position
// =============================================================================

/// Position data from the Data API
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    /// User's proxy wallet address
    pub proxy_wallet: String,

    /// Asset token ID
    pub asset: String,

    /// Market condition ID
    pub condition_id: String,

    /// Position size (number of shares)
    pub size: f64,

    /// Average entry price
    pub avg_price: f64,

    /// Initial position value (entry value)
    pub initial_value: f64,

    /// Current position value
    pub current_value: f64,

    /// Unrealized cash P&L
    pub cash_pnl: f64,

    /// Unrealized percentage P&L
    pub percent_pnl: f64,

    /// Total amount bought
    pub total_bought: f64,

    /// Realized P&L from closed portions
    pub realized_pnl: f64,

    /// Realized P&L as percentage
    pub percent_realized_pnl: f64,

    /// Current market price
    pub cur_price: f64,

    /// Whether position can be redeemed (market resolved)
    pub redeemable: bool,

    /// Whether position can be merged
    pub mergeable: bool,

    /// Market title/question
    pub title: String,

    /// Market URL slug
    pub slug: String,

    /// Position outcome (e.g., "Yes", "No")
    pub outcome: String,

    /// Outcome index (0 or 1)
    pub outcome_index: i32,

    /// Opposite outcome name
    pub opposite_outcome: String,

    /// Opposite outcome asset token ID
    pub opposite_asset: String,

    /// Market end date (ISO 8601)
    pub end_date: String,

    /// Whether market uses negative risk framework
    pub negative_risk: bool,

    // Optional fields that may not always be present
    /// Market icon URL
    #[serde(default)]
    pub icon: Option<String>,

    /// Event ID (numeric ID as string)
    #[serde(default)]
    pub event_id: Option<String>,

    /// Event slug
    #[serde(default)]
    pub event_slug: Option<String>,
}

// =============================================================================
// Filters and Sorting
// =============================================================================

/// Sort field for position queries
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PositionSortBy {
    /// Sort by current value
    Current,
    /// Sort by initial value
    Initial,
    /// Sort by token count (default)
    #[default]
    Tokens,
    /// Sort by cash P&L
    CashPnl,
    /// Sort by percent P&L
    PercentPnl,
    /// Sort by market title
    Title,
    /// Sort by resolving status
    Resolving,
    /// Sort by current price
    Price,
    /// Sort by average entry price
    AvgPrice,
}

impl PositionSortBy {
    /// Convert to API query string value
    pub fn as_str(&self) -> &'static str {
        match self {
            PositionSortBy::Current => "CURRENT",
            PositionSortBy::Initial => "INITIAL",
            PositionSortBy::Tokens => "TOKENS",
            PositionSortBy::CashPnl => "CASHPNL",
            PositionSortBy::PercentPnl => "PERCENTPNL",
            PositionSortBy::Title => "TITLE",
            PositionSortBy::Resolving => "RESOLVING",
            PositionSortBy::Price => "PRICE",
            PositionSortBy::AvgPrice => "AVGPRICE",
        }
    }
}

/// Sort direction for position queries
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortDirection {
    /// Ascending order
    Asc,
    /// Descending order (default)
    #[default]
    Desc,
}

impl SortDirection {
    /// Convert to API query string value
    pub fn as_str(&self) -> &'static str {
        match self {
            SortDirection::Asc => "ASC",
            SortDirection::Desc => "DESC",
        }
    }
}

/// Query filters for position requests
#[derive(Debug, Clone, Default)]
pub struct PositionFilters {
    /// Filter by condition IDs (mutually exclusive with event_id)
    pub market: Option<Vec<String>>,

    /// Filter by event IDs (mutually exclusive with market)
    pub event_id: Option<Vec<i64>>,

    /// Minimum position size filter (default: 1)
    pub size_threshold: Option<f64>,

    /// Filter redeemable positions only
    pub redeemable: Option<bool>,

    /// Filter mergeable positions only
    pub mergeable: Option<bool>,

    /// Results per page (0-500, default: 100)
    pub limit: Option<u32>,

    /// Pagination offset (0-10000)
    pub offset: Option<u32>,

    /// Sort field
    pub sort_by: Option<PositionSortBy>,

    /// Sort direction
    pub sort_direction: Option<SortDirection>,

    /// Filter by market title (max 100 chars)
    pub title: Option<String>,
}

impl PositionFilters {
    /// Create new empty filters
    pub fn new() -> Self {
        Self::default()
    }

    /// Convert filters to query parameters
    pub fn to_query_params(&self) -> Vec<(String, String)> {
        let mut params = Vec::new();

        if let Some(markets) = &self.market {
            if !markets.is_empty() {
                params.push(("market".to_string(), markets.join(",")));
            }
        }

        if let Some(event_ids) = &self.event_id {
            if !event_ids.is_empty() {
                let ids: Vec<String> = event_ids.iter().map(|id| id.to_string()).collect();
                params.push(("eventId".to_string(), ids.join(",")));
            }
        }

        if let Some(threshold) = self.size_threshold {
            params.push(("sizeThreshold".to_string(), threshold.to_string()));
        }

        if let Some(redeemable) = self.redeemable {
            params.push(("redeemable".to_string(), redeemable.to_string()));
        }

        if let Some(mergeable) = self.mergeable {
            params.push(("mergeable".to_string(), mergeable.to_string()));
        }

        if let Some(limit) = self.limit {
            params.push(("limit".to_string(), limit.to_string()));
        }

        if let Some(offset) = self.offset {
            params.push(("offset".to_string(), offset.to_string()));
        }

        if let Some(sort_by) = &self.sort_by {
            params.push(("sortBy".to_string(), sort_by.as_str().to_string()));
        }

        if let Some(direction) = &self.sort_direction {
            params.push(("sortDirection".to_string(), direction.as_str().to_string()));
        }

        if let Some(title) = &self.title {
            params.push(("title".to_string(), title.clone()));
        }

        params
    }

    /// Set limit and return self for builder pattern
    pub fn with_limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }

    /// Set offset and return self for builder pattern
    pub fn with_offset(mut self, offset: u32) -> Self {
        self.offset = Some(offset);
        self
    }

    /// Set size threshold and return self for builder pattern
    pub fn with_size_threshold(mut self, threshold: f64) -> Self {
        self.size_threshold = Some(threshold);
        self
    }

    /// Set market condition IDs and return self for builder pattern
    pub fn with_markets(mut self, condition_ids: Vec<String>) -> Self {
        self.market = Some(condition_ids);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_position_deserialization() {
        // Real API response format
        let json = r#"{
            "proxyWallet": "0x000000000000000000000000000000000000dead",
            "asset": "2719515613422582620337412480873700091173613463422048941551190646073023654521",
            "conditionId": "0xc2d728a0c634f0b453d51e61759041cd054706ca808041a44ed07a7986946479",
            "size": 15000,
            "avgPrice": 0.55,
            "initialValue": 8250,
            "currentValue": 9000,
            "cashPnl": 750,
            "percentPnl": 9.09,
            "totalBought": 8250,
            "realizedPnl": 0,
            "percentRealizedPnl": 0,
            "curPrice": 0.60,
            "redeemable": true,
            "mergeable": false,
            "title": "Olympic Basketball: USA vs. Brazil",
            "slug": "olympic-basketball-usa-vs-brazil",
            "icon": "https://example.com/icon.png",
            "eventId": "11871",
            "eventSlug": "olympic-basketball-usa-vs-brazil",
            "outcome": "Brazil",
            "outcomeIndex": 1,
            "oppositeOutcome": "USA",
            "oppositeAsset": "31905956707945147082248578912350061982363371270525287591561984682339308910362",
            "endDate": "2024-08-06",
            "negativeRisk": false
        }"#;

        let position: Position = serde_json::from_str(json).expect("Failed to deserialize position");

        assert_eq!(position.proxy_wallet, "0x000000000000000000000000000000000000dead");
        assert_eq!(position.size, 15000.0);
        assert_eq!(position.avg_price, 0.55);
        assert_eq!(position.cash_pnl, 750.0);
        assert_eq!(position.outcome, "Brazil");
        assert_eq!(position.outcome_index, 1);
        assert!(position.redeemable);
        assert!(!position.mergeable);
        assert_eq!(position.event_id, Some("11871".to_string()));
        assert_eq!(position.event_slug, Some("olympic-basketball-usa-vs-brazil".to_string()));
    }

    #[test]
    fn test_position_deserialization_minimal() {
        // Test with optional fields missing
        let json = r#"{
            "proxyWallet": "0xabc",
            "asset": "123",
            "conditionId": "0xdef",
            "size": 100,
            "avgPrice": 0.5,
            "initialValue": 50,
            "currentValue": 60,
            "cashPnl": 10,
            "percentPnl": 20,
            "totalBought": 50,
            "realizedPnl": 0,
            "percentRealizedPnl": 0,
            "curPrice": 0.6,
            "redeemable": false,
            "mergeable": false,
            "title": "Test Market",
            "slug": "test-market",
            "outcome": "Yes",
            "outcomeIndex": 0,
            "oppositeOutcome": "No",
            "oppositeAsset": "456",
            "endDate": "2025-01-01",
            "negativeRisk": false
        }"#;

        let position: Position = serde_json::from_str(json).expect("Failed to deserialize position");

        assert_eq!(position.size, 100.0);
        assert_eq!(position.icon, None);
        assert_eq!(position.event_id, None);
        assert_eq!(position.event_slug, None);
    }

    #[test]
    fn test_position_sort_by_as_str() {
        assert_eq!(PositionSortBy::Current.as_str(), "CURRENT");
        assert_eq!(PositionSortBy::Tokens.as_str(), "TOKENS");
        assert_eq!(PositionSortBy::CashPnl.as_str(), "CASHPNL");
    }

    #[test]
    fn test_sort_direction_as_str() {
        assert_eq!(SortDirection::Asc.as_str(), "ASC");
        assert_eq!(SortDirection::Desc.as_str(), "DESC");
    }

    #[test]
    fn test_filters_to_query_params() {
        let filters = PositionFilters {
            size_threshold: Some(10.0),
            limit: Some(50),
            sort_by: Some(PositionSortBy::CashPnl),
            sort_direction: Some(SortDirection::Desc),
            ..Default::default()
        };

        let params = filters.to_query_params();
        assert!(params.iter().any(|(k, v)| k == "sizeThreshold" && v == "10"));
        assert!(params.iter().any(|(k, v)| k == "limit" && v == "50"));
        assert!(params.iter().any(|(k, v)| k == "sortBy" && v == "CASHPNL"));
        assert!(params.iter().any(|(k, v)| k == "sortDirection" && v == "DESC"));
    }

    #[test]
    fn test_filters_builder_pattern() {
        let filters = PositionFilters::new()
            .with_limit(100)
            .with_size_threshold(5.0)
            .with_markets(vec!["cond1".to_string(), "cond2".to_string()]);

        assert_eq!(filters.limit, Some(100));
        assert_eq!(filters.size_threshold, Some(5.0));
        assert_eq!(filters.market, Some(vec!["cond1".to_string(), "cond2".to_string()]));
    }
}
