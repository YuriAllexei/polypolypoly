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

}
