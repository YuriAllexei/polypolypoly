//! Profitability validation.

use crate::application::strategies::inventory_mm::types::InventorySnapshot;

/// Validate if proposed quotes would be profitable
///
/// # Arguments
/// * `best_up_bid` - Best (highest) Up bid we'd place (level 0)
/// * `best_down_bid` - Best (highest) Down bid we'd place (level 0)
/// * `inventory` - Current inventory state
/// * `min_profit_margin` - Minimum required profit per pair
///
/// # Returns
/// true if placing these quotes could lead to profitable merge
pub fn validate_profitability(
    best_up_bid: Option<f64>,
    best_down_bid: Option<f64>,
    inventory: &InventorySnapshot,
    min_profit_margin: f64,
) -> bool {
    // If we have no quotes to place, consider it valid (nothing to check)
    if best_up_bid.is_none() && best_down_bid.is_none() {
        return true;
    }

    // Calculate what combined avg would be after fills
    let projected_up_avg = match best_up_bid {
        Some(bid) => {
            if inventory.up_size > 0.0 {
                // Assume we fill 100 shares at bid price (simplified)
                let fill_size = 100.0;
                let old_cost = inventory.up_size * inventory.up_avg_price;
                let new_cost = fill_size * bid;
                (old_cost + new_cost) / (inventory.up_size + fill_size)
            } else {
                bid
            }
        }
        None => inventory.up_avg_price,
    };

    let projected_down_avg = match best_down_bid {
        Some(bid) => {
            if inventory.down_size > 0.0 {
                let fill_size = 100.0;
                let old_cost = inventory.down_size * inventory.down_avg_price;
                let new_cost = fill_size * bid;
                (old_cost + new_cost) / (inventory.down_size + fill_size)
            } else {
                bid
            }
        }
        None => inventory.down_avg_price,
    };

    // Check if projected combined cost allows for profit
    let combined = projected_up_avg + projected_down_avg;
    let max_combined = 1.0 - min_profit_margin;

    combined < max_combined
}

/// More sophisticated profitability check for multi-level quoting
///
/// Considers:
/// - All levels in the ladder
/// - Probability of fill at each level
/// - Expected avg cost after fills
pub fn validate_profitability_multilevel(
    up_bids: &[(f64, f64)],  // (price, size) for each level
    down_bids: &[(f64, f64)],
    inventory: &InventorySnapshot,
    min_profit_margin: f64,
) -> ProfitabilityResult {
    let mut result = ProfitabilityResult::default();

    // Best case: only level 0 fills on each side
    if let (Some(&(up_price, _)), Some(&(down_price, _))) = (up_bids.first(), down_bids.first()) {
        result.best_case_combined = up_price + down_price;
        result.best_case_profit = 1.0 - result.best_case_combined;
    }

    // Worst case: all levels fill
    if !up_bids.is_empty() && !down_bids.is_empty() {
        let up_total_cost: f64 = up_bids.iter().map(|(p, s)| p * s).sum();
        let up_total_size: f64 = up_bids.iter().map(|(_, s)| s).sum();
        let down_total_cost: f64 = down_bids.iter().map(|(p, s)| p * s).sum();
        let down_total_size: f64 = down_bids.iter().map(|(_, s)| s).sum();

        let worst_up_avg = if up_total_size > 0.0 {
            (inventory.up_size * inventory.up_avg_price + up_total_cost)
                / (inventory.up_size + up_total_size)
        } else {
            inventory.up_avg_price
        };

        let worst_down_avg = if down_total_size > 0.0 {
            (inventory.down_size * inventory.down_avg_price + down_total_cost)
                / (inventory.down_size + down_total_size)
        } else {
            inventory.down_avg_price
        };

        result.worst_case_combined = worst_up_avg + worst_down_avg;
        result.worst_case_profit = 1.0 - result.worst_case_combined;
    }

    // Decision: use worst case for safety
    result.is_profitable = result.worst_case_profit >= min_profit_margin;

    result
}

/// Result of profitability analysis
#[derive(Debug, Default)]
pub struct ProfitabilityResult {
    /// Best case (only best level fills): combined avg cost
    pub best_case_combined: f64,
    /// Best case profit per pair
    pub best_case_profit: f64,
    /// Worst case (all levels fill): combined avg cost
    pub worst_case_combined: f64,
    /// Worst case profit per pair
    pub worst_case_profit: f64,
    /// Is this profitable (based on worst case)?
    pub is_profitable: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_profitable() {
        let inventory = InventorySnapshot {
            up_size: 50.0,
            up_avg_price: 0.52,
            down_size: 50.0,
            down_avg_price: 0.46,
        };

        // Current combined = 0.98, bidding at similar prices keeps it profitable
        let result = validate_profitability(
            Some(0.52),
            Some(0.46),
            &inventory,
            0.01, // need 1 cent profit
        );

        assert!(result);
    }

    #[test]
    fn test_validate_unprofitable() {
        let inventory = InventorySnapshot {
            up_size: 50.0,
            up_avg_price: 0.52,
            down_size: 50.0,
            down_avg_price: 0.47,
        };

        // Current combined = 0.99, bidding higher would make it unprofitable
        let result = validate_profitability(
            Some(0.54),
            Some(0.48),
            &inventory,
            0.01,
        );

        // New avg would be worse, combined > 0.99
        assert!(!result);
    }

    #[test]
    fn test_validate_no_inventory() {
        let inventory = InventorySnapshot::default();

        // No inventory, just check if bids themselves are profitable
        let result = validate_profitability(
            Some(0.52),
            Some(0.46),
            &inventory,
            0.01,
        );

        // 0.52 + 0.46 = 0.98 < 0.99, profitable
        assert!(result);
    }

    #[test]
    fn test_validate_one_side_only() {
        let inventory = InventorySnapshot {
            up_size: 50.0,
            up_avg_price: 0.52,
            down_size: 0.0,
            down_avg_price: 0.0,
        };

        // Only Up bid, no Down
        let result = validate_profitability(
            Some(0.52),
            None,
            &inventory,
            0.01,
        );

        // Can't check profitability without both sides, default to true
        assert!(result);
    }

    #[test]
    fn test_multilevel_profitability() {
        let inventory = InventorySnapshot {
            up_size: 50.0,
            up_avg_price: 0.52,
            down_size: 50.0,
            down_avg_price: 0.46,
        };

        let up_bids = vec![
            (0.54, 100.0), // level 0
            (0.53, 100.0), // level 1
            (0.52, 100.0), // level 2
        ];
        let down_bids = vec![
            (0.44, 100.0),
            (0.43, 100.0),
            (0.42, 100.0),
        ];

        let result = validate_profitability_multilevel(
            &up_bids,
            &down_bids,
            &inventory,
            0.01,
        );

        // Best case: 0.54 + 0.44 = 0.98, profit = 0.02
        assert!((result.best_case_combined - 0.98).abs() < 0.001);
        assert!(result.is_profitable);
    }
}
