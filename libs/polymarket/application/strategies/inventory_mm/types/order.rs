//! Order types for the QuotingSolver

/// Side of an order
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Buy,
    Sell,
}

impl Side {
    pub fn opposite(&self) -> Self {
        match self {
            Side::Buy => Side::Sell,
            Side::Sell => Side::Buy,
        }
    }
}

impl std::fmt::Display for Side {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Side::Buy => write!(f, "BUY"),
            Side::Sell => write!(f, "SELL"),
        }
    }
}

/// A desired quote (price level we want to have an order at)
#[derive(Debug, Clone)]
pub struct Quote {
    pub token_id: String,
    pub price: f64,
    pub size: f64,
    pub side: Side,
    /// Level in the ladder (0 = best/closest to ask)
    pub level: usize,
}

impl Quote {
    pub fn new_bid(token_id: String, price: f64, size: f64, level: usize) -> Self {
        Self {
            token_id,
            price,
            size,
            side: Side::Buy,
            level,
        }
    }
}

/// A limit order to be placed
#[derive(Debug, Clone)]
pub struct LimitOrder {
    pub token_id: String,
    pub price: f64,
    pub size: f64,
    pub side: Side,
}

impl LimitOrder {
    pub fn new(token_id: String, price: f64, size: f64, side: Side) -> Self {
        Self {
            token_id,
            price,
            size,
            side,
        }
    }

    pub fn buy(token_id: String, price: f64, size: f64) -> Self {
        Self::new(token_id, price, size, Side::Buy)
    }

    pub fn sell(token_id: String, price: f64, size: f64) -> Self {
        Self::new(token_id, price, size, Side::Sell)
    }
}

/// Quote ladder for both sides
#[derive(Debug, Clone, Default)]
pub struct QuoteLadder {
    pub up_quotes: Vec<Quote>,
    pub down_quotes: Vec<Quote>,
}

impl QuoteLadder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.up_quotes.is_empty() && self.down_quotes.is_empty()
    }

    pub fn total_quotes(&self) -> usize {
        self.up_quotes.len() + self.down_quotes.len()
    }
}
