use crate::domain::DbMarket;
use crate::infrastructure::FullTimeEvent;
use super::super::types::WinnerAnalysis;
use tracing::info;

/// Log the winning token analysis
pub fn log_winning_token(market: &DbMarket, event: &FullTimeEvent, winner: &Option<WinnerAnalysis>) {
    let market_url = market
        .slug
        .as_ref()
        .map(|s| format!("https://polymarket.com/event/{}", s))
        .unwrap_or_else(|| "N/A".to_string());

    info!("════════════════════════════════════════════════════════════════");
    info!("  🏆 WINNER ANALYSIS - GAME ENDED");
    info!("════════════════════════════════════════════════════════════════");
    info!(
        "  Game: {} vs {}",
        event.home_team.as_deref().unwrap_or("?"),
        event.away_team.as_deref().unwrap_or("?")
    );
    info!("  Final Score: {}", event.final_score);
    info!("  Market: {}", market.question);
    info!("  URL: {}", market_url);

    match winner {
        Some(w) => {
            info!("  ─────────────────────────────────────────────────────────────");
            info!("  Predicted Winner: {}", w.outcome_name);
            info!("  Token ID: {}", w.token_id);
            if let Some((price, size)) = w.best_bid {
                info!("  Best Bid: ${:.4} x {:.2}", price, size);
            } else {
                info!("  Best Bid: None");
            }
            info!("  Has Asks: {}", w.has_asks);
            info!("  Confidence: {:.0}%", w.confidence * 100.0);
        }
        None => {
            info!("  Could not determine winner from orderbooks");
        }
    }
    info!("════════════════════════════════════════════════════════════════");
}
