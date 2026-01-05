//! Smart order diffing with queue priority preservation.
//!
//! Preserves FIFO queue priority when adjusting sizes - older orders at same
//! price get filled first, so we keep them when possible.

use std::collections::{HashMap, HashSet};

use crate::application::strategies::inventory_mm::types::{LimitOrder, OpenOrder, Quote};
use crate::application::strategies::inventory_mm::components::in_flight::price_to_key;

/// Size tolerance: 1% of size OR 0.1, whichever is larger
const SIZE_TOLERANCE_PCT: f64 = 0.01;
const SIZE_TOLERANCE_ABS: f64 = 0.1;

/// Polymarket minimum order size (in shares)
const MIN_ORDER_SIZE: f64 = 5.0;

/// Smart diff: preserves queue priority when adjusting order sizes.
///
/// # Algorithm
/// For each price level:
/// 1. Price not in desired -> Cancel all orders at that price
/// 2. Price is new -> Place new order at desired size
/// 3. Price exists in both (size adjustment):
///    - current_total == desired -> No action
///    - current_total < desired -> Keep all, place additional order for difference
///    - current_total > desired -> Keep oldest orders, cancel rest, place remainder if needed
///
/// # Returns
/// (orders_to_cancel, orders_to_place)
pub fn diff_orders(
    current: &[OpenOrder],
    desired: &[Quote],
    token_id: &str,
) -> (Vec<String>, Vec<LimitOrder>) {
    let mut to_cancel = Vec::new();
    let mut to_place = Vec::new();

    // Group current orders by price level
    let current_by_price = group_orders_by_price(current);
    let desired_by_price = group_quotes_by_price(desired);

    // Get all unique price keys
    let mut all_price_keys: Vec<i64> = current_by_price
        .keys()
        .chain(desired_by_price.keys())
        .copied()
        .collect();
    all_price_keys.sort_unstable();
    all_price_keys.dedup();

    for price_key in all_price_keys {
        let current_orders = current_by_price.get(&price_key);
        let desired_quote = desired_by_price.get(&price_key);

        match (current_orders, desired_quote) {
            // Case 1: Price in current but not desired -> Cancel all
            (Some(orders), None) => {
                for order in orders.iter() {
                    to_cancel.push(order.order_id.clone());
                }
            }

            // Case 2: Price is new -> Place new order (if size meets minimum)
            (None, Some(quote)) => {
                if quote.size >= MIN_ORDER_SIZE {
                    to_place.push(LimitOrder::new(
                        token_id.to_string(),
                        quote.price,
                        quote.size,
                        quote.side,
                    ));
                }
                // Skip quotes with sub-minimum size
            }

            // Case 3: Price exists in both -> Smart size adjustment
            (Some(orders), Some(quote)) => {
                let (cancels, place) = adjust_size_at_price(orders, quote, token_id);
                to_cancel.extend(cancels);
                if let Some(order) = place {
                    to_place.push(order);
                }
            }

            // Case 4: Nothing exists (shouldn't happen but handle gracefully)
            (None, None) => {}
        }
    }

    (to_cancel, to_place)
}

fn group_orders_by_price(orders: &[OpenOrder]) -> HashMap<i64, Vec<&OpenOrder>> {
    let mut map: HashMap<i64, Vec<&OpenOrder>> = HashMap::new();
    for order in orders {
        map.entry(price_to_key(order.price))
            .or_default()
            .push(order);
    }
    map
}

fn group_quotes_by_price(quotes: &[Quote]) -> HashMap<i64, &Quote> {
    use tracing::warn;
    let mut map: HashMap<i64, &Quote> = HashMap::new();
    for q in quotes {
        let key = price_to_key(q.price);
        if let Some(existing) = map.get(&key) {
            // CRITICAL: Duplicate price detected - log warning and keep the larger size
            // This prevents silent data loss when solver generates duplicate prices
            warn!(
                "[diff] Duplicate price {:.4} detected! Existing size={:.1}, new size={:.1} - keeping larger",
                q.price, existing.size, q.size
            );
            if q.size > existing.size {
                map.insert(key, q);
            }
            // Keep existing if it's larger or equal
        } else {
            map.insert(key, q);
        }
    }
    map
}

fn adjust_size_at_price(
    orders: &[&OpenOrder],
    quote: &Quote,
    token_id: &str,
) -> (Vec<String>, Option<LimitOrder>) {
    let desired_size = quote.size;
    let current_total: f64 = orders.iter().map(|o| o.remaining_size).sum();

    let tolerance = (desired_size * SIZE_TOLERANCE_PCT).max(SIZE_TOLERANCE_ABS);

    // Sizes match within tolerance - no action
    if (current_total - desired_size).abs() < tolerance {
        return (vec![], None);
    }

    // Need MORE - keep all, place additional (if above minimum)
    if current_total < desired_size {
        // CRITICAL: Round to whole numbers - Polymarket rejects fractional sizes
        let additional = (desired_size - current_total).round();
        // Only place if additional size meets minimum
        let place_order = if additional >= MIN_ORDER_SIZE {
            Some(LimitOrder::new(
                token_id.to_string(),
                quote.price,
                additional,
                quote.side,
            ))
        } else {
            None  // Skip sub-minimum orders
        };
        return (vec![], place_order);
    }

    // Need LESS - keep oldest orders, cancel rest
    let mut sorted_orders: Vec<&OpenOrder> = orders.to_vec();
    sorted_orders.sort_by_key(|o| o.created_at);

    let mut kept_sum = 0.0;
    let mut kept_ids: HashSet<&str> = HashSet::new();

    for order in &sorted_orders {
        if kept_sum + order.remaining_size <= desired_size + SIZE_TOLERANCE_ABS {
            kept_sum += order.remaining_size;
            kept_ids.insert(&order.order_id);
        } else {
            break;
        }
    }

    let to_cancel: Vec<String> = orders
        .iter()
        .filter(|o| !kept_ids.contains(o.order_id.as_str()))
        .map(|o| o.order_id.clone())
        .collect();

    // CRITICAL: Round to whole numbers - Polymarket rejects fractional sizes
    let remainder = (desired_size - kept_sum).round();
    // Only place if remainder meets minimum size requirement
    let new_order = if remainder >= MIN_ORDER_SIZE {
        Some(LimitOrder::new(
            token_id.to_string(),
            quote.price,
            remainder,
            quote.side,
        ))
    } else {
        None  // Skip sub-minimum orders
    };

    (to_cancel, new_order)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order(id: &str, price: f64, size: f64, created_at: i64) -> OpenOrder {
        OpenOrder::with_created_at(id.to_string(), price, size, size, created_at)
    }

    fn order_simple(id: &str, price: f64, size: f64) -> OpenOrder {
        OpenOrder::new(id.to_string(), price, size, size)
    }

    #[test]
    fn test_diff_no_changes() {
        // Current matches desired exactly -> no changes
        let current = vec![
            order_simple("order1", 0.54, 100.0),
            order_simple("order2", 0.53, 100.0),
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
        // New price level needed -> place new order
        let current = vec![order_simple("order1", 0.54, 100.0)];
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
        // Order at old price -> cancel it, place at new price
        let current = vec![
            order_simple("order1", 0.54, 100.0),
            order_simple("order2", 0.52, 100.0), // stale price
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
        // All prices changed -> cancel all, place all new
        let current = vec![
            order_simple("order1", 0.50, 100.0),
            order_simple("order2", 0.49, 100.0),
        ];
        let desired = vec![
            Quote::new_bid("token".to_string(), 0.54, 100.0, 0),
            Quote::new_bid("token".to_string(), 0.53, 100.0, 1),
        ];

        let (to_cancel, to_place) = diff_orders(&current, &desired, "token");

        assert_eq!(to_cancel.len(), 2);
        assert_eq!(to_place.len(), 2);
    }

    // =========================================================================
    // Queue Priority Preservation Tests
    // =========================================================================

    #[test]
    fn test_queue_priority_decrease_size() {
        // Current: 3 orders @ 0.49 totaling 300, oldest first
        // Desired: 140 @ 0.49
        // Should: keep oldest (100), cancel middle and newest, place 40
        let current = vec![
            order("A", 0.49, 100.0, 1000), // oldest
            order("B", 0.49, 100.0, 1001),
            order("C", 0.49, 100.0, 1002), // newest
        ];
        let desired = vec![Quote::new_bid("token".to_string(), 0.49, 140.0, 0)];

        let (to_cancel, to_place) = diff_orders(&current, &desired, "token");

        // Should cancel B and C (the newer ones)
        assert_eq!(to_cancel.len(), 2);
        assert!(to_cancel.contains(&"B".to_string()));
        assert!(to_cancel.contains(&"C".to_string()));
        assert!(!to_cancel.contains(&"A".to_string())); // A is kept!

        // Should place 40 to make up difference (140 - 100)
        assert_eq!(to_place.len(), 1);
        assert!((to_place[0].size - 40.0).abs() < 0.01);
    }

    #[test]
    fn test_queue_priority_increase_size() {
        // Current: 1 order @ 0.49 for 100
        // Desired: 250 @ 0.49
        // Should: keep existing, place additional 150
        let current = vec![order("A", 0.49, 100.0, 1000)];
        let desired = vec![Quote::new_bid("token".to_string(), 0.49, 250.0, 0)];

        let (to_cancel, to_place) = diff_orders(&current, &desired, "token");

        // Should not cancel anything
        assert!(to_cancel.is_empty());

        // Should place additional 150
        assert_eq!(to_place.len(), 1);
        assert!((to_place[0].size - 150.0).abs() < 0.01);
    }

    #[test]
    fn test_queue_priority_first_order_exceeds_desired() {
        // Current: 1 order @ 0.49 for 150
        // Desired: 100 @ 0.49
        // Cannot fit first order -> cancel all, place new
        let current = vec![order("A", 0.49, 150.0, 1000)];
        let desired = vec![Quote::new_bid("token".to_string(), 0.49, 100.0, 0)];

        let (to_cancel, to_place) = diff_orders(&current, &desired, "token");

        // Should cancel the 150 order
        assert_eq!(to_cancel.len(), 1);
        assert_eq!(to_cancel[0], "A");

        // Should place exact 100
        assert_eq!(to_place.len(), 1);
        assert!((to_place[0].size - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_queue_priority_keep_two_oldest() {
        // Current: 4 orders @ 0.49 totaling 400
        // Desired: 200 @ 0.49
        // Should: keep two oldest (200), cancel two newest
        let current = vec![
            order("A", 0.49, 100.0, 1000), // oldest
            order("B", 0.49, 100.0, 1001),
            order("C", 0.49, 100.0, 1002),
            order("D", 0.49, 100.0, 1003), // newest
        ];
        let desired = vec![Quote::new_bid("token".to_string(), 0.49, 200.0, 0)];

        let (to_cancel, to_place) = diff_orders(&current, &desired, "token");

        // Should cancel C and D
        assert_eq!(to_cancel.len(), 2);
        assert!(to_cancel.contains(&"C".to_string()));
        assert!(to_cancel.contains(&"D".to_string()));
        assert!(!to_cancel.contains(&"A".to_string()));
        assert!(!to_cancel.contains(&"B".to_string()));

        // No new order needed (200 == 200)
        assert!(to_place.is_empty());
    }

    #[test]
    fn test_queue_priority_multiple_price_levels() {
        // Two price levels, different adjustments needed
        let current = vec![
            order("A1", 0.49, 100.0, 1000),
            order("A2", 0.49, 100.0, 1001),
            order("B1", 0.48, 50.0, 1002),
        ];
        let desired = vec![
            Quote::new_bid("token".to_string(), 0.49, 150.0, 0), // decrease from 200 to 150
            Quote::new_bid("token".to_string(), 0.48, 100.0, 1), // increase from 50 to 100
        ];

        let (to_cancel, to_place) = diff_orders(&current, &desired, "token");

        // @ 0.49: keep A1 (100), cancel A2, place 50
        assert!(to_cancel.contains(&"A2".to_string()));
        assert!(!to_cancel.contains(&"A1".to_string()));

        // @ 0.48: keep B1, place additional 50
        assert!(!to_cancel.contains(&"B1".to_string()));

        // Should have placements for both levels
        assert_eq!(to_place.len(), 2);
    }

    #[test]
    fn test_price_to_key() {
        assert_eq!(price_to_key(0.49), 4900);
        assert_eq!(price_to_key(0.5), 5000);
        assert_eq!(price_to_key(0.01), 100);
        assert_eq!(price_to_key(0.999), 9990);
    }

    #[test]
    fn test_size_within_tolerance_no_action() {
        // Current: 99.5, Desired: 100 -> diff of 0.5 < tolerance of 1.0, no action
        let current = vec![order("A", 0.49, 99.5, 1000)];
        let desired = vec![Quote::new_bid("token".to_string(), 0.49, 100.0, 0)];

        let (to_cancel, to_place) = diff_orders(&current, &desired, "token");

        assert!(to_cancel.is_empty());
        assert!(to_place.is_empty());
    }
}
