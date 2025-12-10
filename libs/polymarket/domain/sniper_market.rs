//! Sniper market data extraction and display

use crate::domain::models::DbMarket;
use anyhow::Result;
use chrono::{DateTime, Utc};
use tracing::info;

/// Represents a market ready for sniping with parsed data
#[derive(Clone)]
pub struct SniperMarket {
    pub id: String,
    pub question: String,
    pub slug: Option<String>,
    pub resolution_time: DateTime<Utc>,
    pub resolution_time_str: String,
    pub token_ids: Vec<String>,
    pub outcomes: Vec<String>,
    pub active: bool,
    pub closed: bool,
    pub liquidity: Option<String>,
    pub volume: Option<String>,
}

impl SniperMarket {
    /// Extract market data from database market
    pub fn from_db_market(market: &DbMarket) -> Result<Self> {
        let resolution_time = market.resolution_datetime()?;
        let token_ids = market.parse_token_ids().unwrap_or_default();
        let outcomes = Self::parse_outcomes(&market.outcomes);

        Ok(Self {
            id: market.id.clone(),
            question: market.question.clone(),
            slug: market.slug.clone(),
            resolution_time,
            resolution_time_str: market.resolution_time.clone(),
            token_ids,
            outcomes,
            active: market.active,
            closed: market.closed,
            liquidity: market.liquidity.clone(),
            volume: market.volume.clone(),
        })
    }

    /// Parse double-encoded outcomes from database
    /// The outcomes field is stored as a JSON-encoded string containing a JSON array
    fn parse_outcomes(outcomes_json: &str) -> Vec<String> {
        // First try double-encoded (JSON string containing JSON array)
        serde_json::from_str::<String>(outcomes_json)
            .ok()
            .and_then(|inner| serde_json::from_str(&inner).ok())
            // Fallback to single-encoded JSON array
            .or_else(|| serde_json::from_str(outcomes_json).ok())
            .unwrap_or_default()
    }

    /// Calculate time until resolution
    pub fn time_until_resolution(&self) -> String {
        let now = Utc::now();
        let duration = self.resolution_time.signed_duration_since(now);
        if duration.num_seconds() > 0 {
            format!("{} seconds", duration.num_seconds())
        } else {
            "Expired".to_string()
        }
    }

    /// Check if market can spawn a tracker
    pub fn can_spawn_tracker(&self) -> bool {
        !self.token_ids.is_empty() && !self.outcomes.is_empty()
    }

    /// Log market details
    pub fn log(&self, iteration: u64) {
        info!("========================================");
        info!("NEW MARKET FOUND (Iteration #{})", iteration);
        info!("========================================");
        info!("  ID: {}", self.id);
        info!("  Question: {}", self.question);
        info!("  Resolution Time: {}", self.resolution_time.format("%Y-%m-%d %H:%M:%S UTC"));
        info!("  Time Until Resolution: {}", self.time_until_resolution());
        info!("  Active: {}", self.active);
        info!("  Closed: {}", self.closed);

        if let Some(liquidity) = &self.liquidity {
            info!("  Liquidity: {}", liquidity);
        }
        if let Some(volume) = &self.volume {
            info!("  Volume: {}", volume);
        }

        self.log_outcomes_and_tokens();
        info!("========================================");
        info!("");
    }

    fn log_outcomes_and_tokens(&self) {
        if !self.outcomes.is_empty() && !self.token_ids.is_empty() {
            info!("  Outcomes:");
            for (idx, outcome) in self.outcomes.iter().enumerate() {
                if let Some(token_id) = self.token_ids.get(idx) {
                    info!("    [{}] {} -> Token ID: {}", idx, outcome, token_id);
                } else {
                    info!("    [{}] {}", idx, outcome);
                }
            }
        } else if !self.outcomes.is_empty() {
            info!("  Outcomes: {:?}", self.outcomes);
        } else if !self.token_ids.is_empty() {
            info!("  Token IDs: {:?}", self.token_ids);
        }
    }
}
