use crate::infrastructure::SharedOrderbooks;
use super::super::types::WinnerAnalysis;

/// Analyze orderbooks to find the likely winning token
///
/// Winner criteria:
/// - Highest bid price
/// - Preferably no asks (market makers pulled out)
/// - High confidence if bid > 0.90 and no asks
pub fn analyze_orderbooks_for_winner(
    orderbooks: &SharedOrderbooks,
    token_ids: &[String],
    outcomes: &[String],
) -> Option<WinnerAnalysis> {
    let obs = orderbooks.read();
    let mut best_candidate: Option<WinnerAnalysis> = None;

    for (token_id, outcome) in token_ids.iter().zip(outcomes.iter()) {
        if let Some(ob) = obs.get(token_id) {
            let best_bid = ob.best_bid();
            let has_asks = !ob.asks.is_empty();

            // Winner criteria: highest bid price
            let is_better = match &best_candidate {
                None => true,
                Some(current) => match (best_bid, current.best_bid) {
                    (Some((price, _)), Some((curr_price, _))) => price > curr_price,
                    (Some(_), None) => true,
                    _ => false,
                },
            };

            if is_better {
                // Calculate confidence based on bid price and ask presence
                let confidence = match best_bid {
                    Some((price, _)) if !has_asks && price > 0.90 => 1.0,
                    Some((price, _)) if !has_asks && price > 0.70 => 0.8,
                    Some((price, _)) if price > 0.90 => 0.7,
                    Some(_) => 0.5,
                    None => 0.1,
                };

                best_candidate = Some(WinnerAnalysis {
                    token_id: token_id.clone(),
                    outcome_name: outcome.clone(),
                    best_bid,
                    has_asks,
                    confidence,
                });
            }
        }
    }

    best_candidate
}
