//! Order diffing logic.

use crate::application::strategies::inventory_mm::types::{
    Quote, LimitOrder, OpenOrder,
};

/// Compare current orders with desired quotes
/// Returns (orders_to_cancel, orders_to_place)
///
/// # Algorithm
/// 1. For each current order, check if there's a matching desired quote
/// 2. If no match, add to cancellation list
/// 3. For each desired quote, check if there's a matching current order
/// 4. If no match, add to placement list
///
/// # Matching criteria
/// - Price within tick tolerance (0.0001)
/// - Size approximately equal (within 1%)
pub fn diff_orders(
    current: &[OpenOrder],
    desired: &[Quote],
    token_id: &str,
) -> (Vec<String>, Vec<LimitOrder>) {
    let mut to_cancel = Vec::new();
    let mut to_place = Vec::new();

    // Find orders to cancel (in current but not in desired)
    for order in current {
        let has_match = desired.iter().any(|q| orders_match(order, q));
        if !has_match {
            to_cancel.push(order.order_id.clone());
        }
    }

    // Find quotes to place (in desired but not in current)
    for quote in desired {
        let has_match = current.iter().any(|o| orders_match(o, quote));
        if !has_match {
            to_place.push(LimitOrder::new(
                token_id.to_string(),
                quote.price,
                quote.size,
                quote.side,
            ));
        }
    }

    (to_cancel, to_place)
}

/// Check if an existing order matches a desired quote
fn orders_match(order: &OpenOrder, quote: &Quote) -> bool {
    // Price must be within tick tolerance
    let price_match = (order.price - quote.price).abs() < 0.0001;

    // Size must be approximately equal (within 1% or 0.1, whichever is larger)
    let size_tolerance = (quote.size * 0.01).max(0.1);
    let size_match = (order.remaining_size - quote.size).abs() < size_tolerance;

    price_match && size_match
}

/// More sophisticated diff that considers partial fills
/// Returns actions needed to reconcile current state with desired
pub fn diff_orders_advanced(
    current: &[OpenOrder],
    desired: &[Quote],
    token_id: &str,
) -> DiffResult {
    let mut result = DiffResult::new();

    // Build map of desired prices for quick lookup
    let desired_by_price: std::collections::HashMap<i64, &Quote> = desired
        .iter()
        .map(|q| (price_to_key(q.price), q))
        .collect();

    // Check each current order
    for order in current {
        let key = price_to_key(order.price);
        match desired_by_price.get(&key) {
            Some(quote) => {
                // Price matches - check size
                let size_diff = quote.size - order.remaining_size;
                if size_diff.abs() > 0.1 {
                    // Size mismatch - need to cancel and re-place
                    result.to_cancel.push(order.order_id.clone());
                    result.to_place.push(LimitOrder::new(
                        token_id.to_string(),
                        quote.price,
                        quote.size,
                        quote.side,
                    ));
                } else {
                    // Order is good, keep it
                    result.unchanged += 1;
                }
            }
            None => {
                // No matching desired quote - cancel
                result.to_cancel.push(order.order_id.clone());
            }
        }
    }

    // Build map of current prices
    let current_by_price: std::collections::HashMap<i64, &OpenOrder> = current
        .iter()
        .map(|o| (price_to_key(o.price), o))
        .collect();

    // Check each desired quote for ones that need placement
    for quote in desired {
        let key = price_to_key(quote.price);
        if !current_by_price.contains_key(&key) {
            // No current order at this price - place new
            result.to_place.push(LimitOrder::new(
                token_id.to_string(),
                quote.price,
                quote.size,
                quote.side,
            ));
        }
    }

    result
}

/// Result of diff operation
#[derive(Debug, Default)]
pub struct DiffResult {
    pub to_cancel: Vec<String>,
    pub to_place: Vec<LimitOrder>,
    /// Orders that are already correct (for logging/metrics)
    pub unchanged: usize,
}

impl DiffResult {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn has_changes(&self) -> bool {
        !self.to_cancel.is_empty() || !self.to_place.is_empty()
    }
}

/// Convert price to integer key for HashMap (avoid float comparison issues)
fn price_to_key(price: f64) -> i64 {
    (price * 10000.0).round() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diff_no_changes() {
        let current = vec![
            OpenOrder::new("order1".to_string(), 0.54, 100.0, 100.0),
            OpenOrder::new("order2".to_string(), 0.53, 100.0, 100.0),
        ];
        let desired = vec![
            Quote::new_bid("token".to_string(), 0.54, 100.0, 0),
            Quote::new_bid("token".to_string(), 0.53, 100.0, 1),
        ];

        let (to_cancel, to_place) = diff_orders(&current, &desired, "token");

        assert!(to_cancel.is_empty());
        assert!(to_place.is_empty());
    }

    #[test]
    fn test_diff_add_new_level() {
        let current = vec![
            OpenOrder::new("order1".to_string(), 0.54, 100.0, 100.0),
        ];
        let desired = vec![
            Quote::new_bid("token".to_string(), 0.54, 100.0, 0),
            Quote::new_bid("token".to_string(), 0.53, 100.0, 1),
        ];

        let (to_cancel, to_place) = diff_orders(&current, &desired, "token");

        assert!(to_cancel.is_empty());
        assert_eq!(to_place.len(), 1);
        assert!((to_place[0].price - 0.53).abs() < 0.001);
    }

    #[test]
    fn test_diff_cancel_stale_order() {
        let current = vec![
            OpenOrder::new("order1".to_string(), 0.54, 100.0, 100.0),
            OpenOrder::new("order2".to_string(), 0.52, 100.0, 100.0), // stale
        ];
        let desired = vec![
            Quote::new_bid("token".to_string(), 0.54, 100.0, 0),
            Quote::new_bid("token".to_string(), 0.53, 100.0, 1),
        ];

        let (to_cancel, to_place) = diff_orders(&current, &desired, "token");

        assert_eq!(to_cancel.len(), 1);
        assert_eq!(to_cancel[0], "order2");
        assert_eq!(to_place.len(), 1);
        assert!((to_place[0].price - 0.53).abs() < 0.001);
    }

    #[test]
    fn test_diff_full_replacement() {
        let current = vec![
            OpenOrder::new("order1".to_string(), 0.50, 100.0, 100.0),
            OpenOrder::new("order2".to_string(), 0.49, 100.0, 100.0),
        ];
        let desired = vec![
            Quote::new_bid("token".to_string(), 0.54, 100.0, 0),
            Quote::new_bid("token".to_string(), 0.53, 100.0, 1),
        ];

        let (to_cancel, to_place) = diff_orders(&current, &desired, "token");

        assert_eq!(to_cancel.len(), 2);
        assert_eq!(to_place.len(), 2);
    }

    #[test]
    fn test_price_to_key() {
        assert_eq!(price_to_key(0.54), 5400);
        assert_eq!(price_to_key(0.5401), 5401);
        assert_eq!(price_to_key(0.99), 9900);
    }
}
