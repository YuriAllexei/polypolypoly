//! Quote types for the Market Merger strategy

/// A single quote (bid) to be placed
#[derive(Debug, Clone)]
pub struct Quote {
    /// Token ID to place bid on
    pub token_id: String,
    /// Bid price
    pub price: f64,
    /// Bid size (in tokens)
    pub size: f64,
    /// Bid level (0, 1, 2)
    pub level: u8,
}

/// Multi-level bid ladder for both tokens
#[derive(Debug, Clone, Default)]
pub struct QuoteLadder {
    /// Bid levels for Up token (typically 3 levels)
    pub up_bids: Vec<Quote>,
    /// Bid levels for Down token (typically 3 levels)
    pub down_bids: Vec<Quote>,
}

impl QuoteLadder {
    /// Create a new empty ladder
    pub fn new() -> Self {
        Self {
            up_bids: Vec::new(),
            down_bids: Vec::new(),
        }
    }

    /// Check if the ladder is empty (no bids)
    pub fn is_empty(&self) -> bool {
        self.up_bids.is_empty() && self.down_bids.is_empty()
    }

    /// Total number of bids in the ladder
    pub fn len(&self) -> usize {
        self.up_bids.len() + self.down_bids.len()
    }

    /// Get all quotes as a flat iterator
    pub fn all_quotes(&self) -> impl Iterator<Item = &Quote> {
        self.up_bids.iter().chain(self.down_bids.iter())
    }

    /// Get total USD value of all bids
    pub fn total_value(&self) -> f64 {
        self.all_quotes().map(|q| q.price * q.size).sum()
    }
}

/// A taker opportunity identified by the opportunity scanner
#[derive(Debug, Clone)]
pub struct TakerOpportunity {
    /// Token ID to take
    pub token_id: String,
    /// Whether this is the Up token
    pub is_up: bool,
    /// Ask price to take
    pub price: f64,
    /// Size to take (in tokens)
    pub size: f64,
    /// Opportunity score (higher = better)
    pub score: f64,
    /// Reasons contributing to the score (for logging)
    pub reasons: Vec<String>,
}

impl TakerOpportunity {
    /// Create a new taker opportunity
    pub fn new(token_id: String, is_up: bool, price: f64, size: f64) -> Self {
        Self {
            token_id,
            is_up,
            price,
            size,
            score: 0.0,
            reasons: Vec::new(),
        }
    }

    /// Add score contribution with reason
    pub fn add_score(&mut self, points: f64, reason: &str) {
        self.score += points;
        self.reasons.push(reason.to_string());
    }

    /// Check if opportunity meets minimum score threshold
    pub fn is_viable(&self, min_score: f64) -> bool {
        self.score >= min_score
    }
}

impl std::fmt::Display for TakerOpportunity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let side = if self.is_up { "Up" } else { "Down" };
        write!(
            f,
            "{} @ ${:.3} x{:.1} (score: {:.1}, reasons: {})",
            side,
            self.price,
            self.size,
            self.score,
            self.reasons.join(", ")
        )
    }
}

impl Quote {
    /// Create a new quote
    pub fn new(token_id: String, price: f64, size: f64, level: u8) -> Self {
        Self {
            token_id,
            price,
            size,
            level,
        }
    }

    /// USD value of this quote
    pub fn value(&self) -> f64 {
        self.price * self.size
    }
}
