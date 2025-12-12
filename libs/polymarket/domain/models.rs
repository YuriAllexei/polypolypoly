use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Database representation of a market
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct DbMarket {
    pub id: String,
    pub condition_id: Option<String>,
    pub question: String,
    pub description: Option<String>, // Inherited from parent event
    pub slug: Option<String>,
    pub start_date: String,      // ISO 8601
    pub end_date: String,        // ISO 8601
    pub resolution_time: String, // ISO 8601
    pub active: bool,
    pub closed: bool,
    pub archived: bool,
    pub market_type: Option<String>,
    pub category: Option<String>,
    pub liquidity: Option<String>,
    pub volume: Option<String>,
    pub outcomes: String,     // JSON array
    pub token_ids: String,    // JSON array
    pub tags: Option<String>, // JSON array of tag objects
    pub last_updated: String, // ISO 8601
    pub created_at: String,   // ISO 8601
}

impl DbMarket {
    /// Convert to DateTime for resolution_time
    pub fn resolution_datetime(&self) -> Result<DateTime<Utc>, chrono::ParseError> {
        DateTime::parse_from_rfc3339(&self.resolution_time).map(|dt| dt.with_timezone(&Utc))
    }

    /// Get outcomes as Vec
    /// Handles both single-encoded and double-encoded JSON strings
    pub fn parse_outcomes(&self) -> Result<Vec<String>, serde_json::Error> {
        // First try double-encoded (JSON string containing JSON array)
        // e.g., "\"[\\\"Up\\\", \\\"Down\\\"]\""
        if let Ok(inner) = serde_json::from_str::<String>(&self.outcomes) {
            if let Ok(vec) = serde_json::from_str::<Vec<String>>(&inner) {
                return Ok(vec);
            }
        }
        // Fallback to single-encoded JSON array
        serde_json::from_str(&self.outcomes)
    }

    /// Get token IDs as Vec
    /// Handles both single-encoded and double-encoded JSON strings
    pub fn parse_token_ids(&self) -> Result<Vec<String>, serde_json::Error> {
        // First try double-encoded (JSON string containing JSON array)
        if let Ok(inner) = serde_json::from_str::<String>(&self.token_ids) {
            if let Ok(vec) = serde_json::from_str::<Vec<String>>(&inner) {
                return Ok(vec);
            }
        }
        // Fallback to single-encoded JSON array
        serde_json::from_str(&self.token_ids)
    }

    /// Get tags as JSON Value
    pub fn parse_tags(&self) -> Result<serde_json::Value, serde_json::Error> {
        match &self.tags {
            Some(tags) => serde_json::from_str(tags),
            None => Ok(serde_json::Value::Array(vec![])),
        }
    }
}

/// Database representation of an event
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct DbEvent {
    pub id: String,
    pub ticker: Option<String>,
    pub slug: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub start_date: Option<String>, // ISO 8601
    pub end_date: Option<String>,   // ISO 8601
    pub active: bool,
    pub closed: bool,
    pub archived: bool,
    pub featured: bool,
    pub restricted: bool,
    pub liquidity: Option<String>,
    pub volume: Option<String>,
    pub volume_24hr: Option<String>,
    pub volume_1wk: Option<String>,
    pub volume_1mo: Option<String>,
    pub volume_1yr: Option<String>,
    pub open_interest: Option<String>,
    pub image: Option<String>,
    pub icon: Option<String>,
    pub category: Option<String>,
    pub competitive: Option<String>,
    pub tags: Option<String>, // JSON array of tag objects
    pub comment_count: i64,
    pub created_at: String,  // ISO 8601
    pub updated_at: String,  // ISO 8601
    pub last_synced: String, // ISO 8601
}

impl DbEvent {
    /// Convert end_date to DateTime
    pub fn end_datetime(&self) -> Option<Result<DateTime<Utc>, chrono::ParseError>> {
        self.end_date
            .as_ref()
            .map(|date| DateTime::parse_from_rfc3339(date).map(|dt| dt.with_timezone(&Utc)))
    }

    /// Convert start_date to DateTime
    pub fn start_datetime(&self) -> Option<Result<DateTime<Utc>, chrono::ParseError>> {
        self.start_date
            .as_ref()
            .map(|date| DateTime::parse_from_rfc3339(date).map(|dt| dt.with_timezone(&Utc)))
    }

    /// Get tags as JSON Value
    pub fn parse_tags(&self) -> Result<serde_json::Value, serde_json::Error> {
        match &self.tags {
            Some(tags) => serde_json::from_str(tags),
            None => Ok(serde_json::Value::Array(vec![])),
        }
    }
}

/// Statistics about sync operation
#[derive(Debug, Clone)]
pub struct SyncStats {
    pub markets_fetched: usize,
    pub markets_inserted: usize,
    pub markets_updated: usize,
    pub duration: std::time::Duration,
}

/// Query filters for markets
#[derive(Debug, Clone, Default)]
pub struct MarketFilters {
    pub active: Option<bool>,
    pub closed: Option<bool>,
    pub archived: Option<bool>,
    pub min_resolution_time: Option<DateTime<Utc>>,
    pub max_resolution_time: Option<DateTime<Utc>>,
    pub category: Option<String>,
}

impl MarketFilters {
    /// Build WHERE clause for SQL query
    pub fn build_where_clause(&self) -> (String, Vec<String>) {
        let mut conditions = Vec::new();
        let mut params = Vec::new();
        let mut idx = 1;

        if let Some(active) = self.active {
            conditions.push(format!("active = ${}", idx));
            params.push(if active { "true" } else { "false" }.to_string());
            idx += 1;
        }

        if let Some(closed) = self.closed {
            conditions.push(format!("closed = ${}", idx));
            params.push(if closed { "true" } else { "false" }.to_string());
            idx += 1;
        }

        if let Some(archived) = self.archived {
            conditions.push(format!("archived = ${}", idx));
            params.push(if archived { "true" } else { "false" }.to_string());
            idx += 1;
        }

        if let Some(min_time) = self.min_resolution_time {
            conditions.push(format!("resolution_time >= ${}", idx));
            params.push(min_time.to_rfc3339());
            idx += 1;
        }

        if let Some(max_time) = self.max_resolution_time {
            conditions.push(format!("resolution_time <= ${}", idx));
            params.push(max_time.to_rfc3339());
            idx += 1;
        }

        if let Some(ref category) = self.category {
            conditions.push(format!("category = ${}", idx));
            params.push(category.clone());
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        (where_clause, params)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_market_filters() {
        let filters = MarketFilters {
            active: Some(true),
            closed: Some(false),
            ..Default::default()
        };

        let (clause, params) = filters.build_where_clause();
        assert!(clause.contains("active = ?"));
        assert!(clause.contains("closed = ?"));
        assert_eq!(params.len(), 2);
    }

    #[test]
    fn test_parse_outcomes_single_encoded() {
        let market = DbMarket {
            id: "test".to_string(),
            condition_id: Some("0x123".to_string()),
            question: "Test?".to_string(),
            description: None,
            slug: None,
            start_date: "2025-01-01T00:00:00Z".to_string(),
            end_date: "2025-01-02T00:00:00Z".to_string(),
            resolution_time: "2025-01-02T00:00:00Z".to_string(),
            active: true,
            closed: false,
            archived: false,
            market_type: None,
            category: None,
            liquidity: None,
            volume: None,
            outcomes: r#"["Yes","No"]"#.to_string(),
            token_ids: r#"["0x1","0x2"]"#.to_string(),
            tags: None,
            last_updated: "2025-01-01T00:00:00Z".to_string(),
            created_at: "2025-01-01T00:00:00Z".to_string(),
        };

        let outcomes = market.parse_outcomes().unwrap();
        assert_eq!(outcomes, vec!["Yes", "No"]);

        let token_ids = market.parse_token_ids().unwrap();
        assert_eq!(token_ids, vec!["0x1", "0x2"]);
    }

    #[test]
    fn test_parse_outcomes_double_encoded() {
        // Double-encoded: the JSON array is itself stored as a JSON string
        let market = DbMarket {
            id: "test".to_string(),
            condition_id: Some("0x123".to_string()),
            question: "Test?".to_string(),
            description: None,
            slug: None,
            start_date: "2025-01-01T00:00:00Z".to_string(),
            end_date: "2025-01-02T00:00:00Z".to_string(),
            resolution_time: "2025-01-02T00:00:00Z".to_string(),
            active: true,
            closed: false,
            archived: false,
            market_type: None,
            category: None,
            liquidity: None,
            volume: None,
            outcomes: r#""[\"Up\", \"Down\"]""#.to_string(),
            token_ids: r#""[\"0xabc\", \"0xdef\"]""#.to_string(),
            tags: None,
            last_updated: "2025-01-01T00:00:00Z".to_string(),
            created_at: "2025-01-01T00:00:00Z".to_string(),
        };

        let outcomes = market.parse_outcomes().unwrap();
        assert_eq!(outcomes, vec!["Up", "Down"]);

        let token_ids = market.parse_token_ids().unwrap();
        assert_eq!(token_ids, vec!["0xabc", "0xdef"]);
    }
}
