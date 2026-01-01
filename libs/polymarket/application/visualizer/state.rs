//! State management for the visualizer

use std::collections::HashMap;

/// Information about a market we're active in
#[derive(Debug, Clone)]
pub struct MarketInfo {
    pub condition_id: String,
    pub market_id: String,
    pub question: String,
    pub up_token_id: String,
    pub down_token_id: String,
    pub up_outcome: String,
    pub down_outcome: String,
    pub display_name: String,
}

impl MarketInfo {
    pub fn new(
        condition_id: String,
        market_id: String,
        question: String,
        up_token_id: String,
        down_token_id: String,
        up_outcome: String,
        down_outcome: String,
    ) -> Self {
        // Create a short display name from the question
        let display_name = Self::create_display_name(&question);

        Self {
            condition_id,
            market_id,
            question,
            up_token_id,
            down_token_id,
            up_outcome,
            down_outcome,
            display_name,
        }
    }

    /// Create a short display name from the question
    fn create_display_name(question: &str) -> String {
        // Try to extract a meaningful short name
        // e.g., "Will BTC be above $100k by..." -> "BTC-100k"
        if question.len() <= 20 {
            return question.to_string();
        }

        // Just truncate for now
        format!("{}...", &question[..17])
    }

    /// Get a shortened version of the question for display
    pub fn question_short(&self) -> String {
        if self.question.len() <= 50 {
            self.question.clone()
        } else {
            format!("{}...", &self.question[..47])
        }
    }
}

/// State shared across the visualizer
pub struct VisualizerState {
    /// Markets discovered from orders/positions
    pub markets: HashMap<String, MarketInfo>,
}

impl VisualizerState {
    pub fn new() -> Self {
        Self {
            markets: HashMap::new(),
        }
    }

    pub fn add_market(&mut self, market: MarketInfo) {
        self.markets.insert(market.condition_id.clone(), market);
    }

    pub fn get_market(&self, condition_id: &str) -> Option<&MarketInfo> {
        self.markets.get(condition_id)
    }
}

impl Default for VisualizerState {
    fn default() -> Self {
        Self::new()
    }
}
