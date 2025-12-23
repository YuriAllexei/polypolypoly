//! Market context for the Market Merger strategy

use crate::application::strategies::up_or_down::{CryptoAsset, Timeframe};
use crate::domain::DbMarket;
use chrono::{DateTime, Utc};

/// Immutable context for a market being accumulated
#[derive(Debug, Clone)]
pub struct MarketContext {
    /// Market ID (slug)
    pub market_id: String,
    /// Condition ID for merging
    pub condition_id: String,
    /// Up token ID
    pub up_token_id: String,
    /// Down token ID
    pub down_token_id: String,
    /// Tick size for price rounding
    pub tick_size: f64,
    /// Decimal precision for prices
    pub precision: u8,
    /// Market end time
    pub market_end_time: DateTime<Utc>,
    /// Crypto asset being tracked
    pub crypto_asset: CryptoAsset,
    /// Timeframe of the market
    pub timeframe: Timeframe,
    /// Market question for logging
    pub market_question: String,
}

impl MarketContext {
    /// Create context from a database market record
    pub fn from_market(market: &DbMarket) -> anyhow::Result<Self> {
        let token_ids = market.parse_token_ids()?;
        let outcomes = market.parse_outcomes()?;

        // Determine which token is Up and which is Down
        let (up_token_id, down_token_id) = Self::identify_tokens(&token_ids, &outcomes)?;

        // Parse end time
        let market_end_time = DateTime::parse_from_rfc3339(&market.end_date)
            .map(|dt| dt.with_timezone(&Utc))
            .map_err(|e| anyhow::anyhow!("Failed to parse end_date: {}", e))?;

        // Detect crypto asset from tags
        let tags = market.parse_tags().unwrap_or_default();
        let crypto_asset = CryptoAsset::from_tags(&tags);

        // Detect timeframe from tags
        let timeframe = Timeframe::from_tags(&tags);

        // Default tick size (will be updated from orderbook)
        let tick_size = 0.01;
        let precision = 2;

        Ok(Self {
            market_id: market.id.clone(),
            condition_id: market.condition_id.clone().unwrap_or_default(),
            up_token_id,
            down_token_id,
            tick_size,
            precision,
            market_end_time,
            crypto_asset,
            timeframe,
            market_question: market.question.clone(),
        })
    }

    /// Identify which token is Up and which is Down based on outcomes
    fn identify_tokens(token_ids: &[String], outcomes: &[String]) -> anyhow::Result<(String, String)> {
        if token_ids.len() != 2 || outcomes.len() != 2 {
            return Err(anyhow::anyhow!(
                "Expected exactly 2 tokens/outcomes, got {} tokens and {} outcomes",
                token_ids.len(),
                outcomes.len()
            ));
        }

        let mut up_token = None;
        let mut down_token = None;

        for (i, outcome) in outcomes.iter().enumerate() {
            let outcome_lower = outcome.to_lowercase();
            if outcome_lower.contains("up") || outcome_lower.contains("yes") {
                up_token = Some(token_ids[i].clone());
            } else if outcome_lower.contains("down") || outcome_lower.contains("no") {
                down_token = Some(token_ids[i].clone());
            }
        }

        match (up_token, down_token) {
            (Some(up), Some(down)) => Ok((up, down)),
            _ => {
                // Fallback: first token is Up, second is Down
                Ok((token_ids[0].clone(), token_ids[1].clone()))
            }
        }
    }

    /// Get outcome name for a token ID
    pub fn get_outcome_name(&self, token_id: &str) -> &str {
        if token_id == self.up_token_id {
            "Up"
        } else if token_id == self.down_token_id {
            "Down"
        } else {
            "Unknown"
        }
    }

    /// Check if this is the Up token
    pub fn is_up_token(&self, token_id: &str) -> bool {
        token_id == self.up_token_id
    }

    /// Check if this is the Down token
    pub fn is_down_token(&self, token_id: &str) -> bool {
        token_id == self.down_token_id
    }

    /// Update tick size and precision from orderbook data
    pub fn update_tick_size(&mut self, tick_size: f64) {
        self.tick_size = tick_size;
        self.precision = crate::infrastructure::decimal_places(&tick_size.to_string());
    }
}
