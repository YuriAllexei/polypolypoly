/// Result of analyzing orderbooks to determine the winning token
#[derive(Debug)]
pub struct WinnerAnalysis {
    pub token_id: String,
    pub outcome_name: String,
    pub best_bid: Option<(f64, f64)>, // (price, size)
    pub has_asks: bool,
    pub confidence: f64, // 0.0 - 1.0
}
