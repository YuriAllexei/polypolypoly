//! Profitability validation for quoting.
//!
//! Ensures quotes are capped at prices that would lead to profitable merges.
//! Key formula: combined_avg_cost = up_avg + down_avg must be < 1.0 - margin
//!
//! For each side:
//! - max_up_bid = 1.0 - down_avg_price - margin (if we have DOWN)
//! - max_down_bid = 1.0 - up_avg_price - margin (if we have UP)
//!
//! Recovery mode: When underwater, relax margins on the needed side to escape.

use tracing::debug;

use crate::application::strategies::inventory_mm::types::{InventorySnapshot, OrderbookSnapshot};

/// Recovery status for underwater positions
#[derive(Debug, Clone)]
pub struct RecoveryStatus {
    /// Whether we're in recovery mode (combined_avg >= recovery_threshold)
    pub in_recovery: bool,
    /// Whether we're stuck (combined_avg >= stuck_threshold)
    pub is_stuck: bool,
    /// Current combined average cost
    pub combined_avg: f64,
}

/// Check if we're in recovery mode based on inventory.
///
/// Recovery mode activates when combined_avg >= recovery_threshold.
/// Stuck mode activates when combined_avg >= stuck_threshold.
pub fn check_recovery_status(
    inventory: &InventorySnapshot,
    recovery_threshold: f64,
    stuck_threshold: f64,
) -> RecoveryStatus {
    let combined = inventory.combined_avg_cost();

    // Only check if we have both sides with meaningful inventory
    if inventory.up_size <= 0.0 || inventory.down_size <= 0.0 {
        return RecoveryStatus {
            in_recovery: false,
            is_stuck: false,
            combined_avg: combined,
        };
    }

    RecoveryStatus {
        in_recovery: combined >= recovery_threshold,
        is_stuck: combined >= stuck_threshold,
        combined_avg: combined,
    }
}

/// Calculate the maximum profitable bid prices for each side.
///
/// Returns (max_up_bid, max_down_bid) - prices above these would be unprofitable.
/// Returns None for a side if we can't determine profitability (no inventory or orderbook data).
///
/// In recovery mode, the margin is relaxed on the NEEDED side only:
/// - If delta > 0 (heavy UP, need DOWN): relax DOWN margin
/// - If delta < 0 (heavy DOWN, need UP): relax UP margin
pub fn calculate_max_bids(
    inventory: &InventorySnapshot,
    up_orderbook: &OrderbookSnapshot,
    down_orderbook: &OrderbookSnapshot,
    min_profit_margin: f64,
    recovery_relaxation: f64,
    delta: f64,
) -> (Option<f64>, Option<f64>) {
    // In recovery, relax margin on the NEEDED side only
    let up_margin = if delta < 0.0 {
        // Need UP (heavy DOWN), relax UP margin
        (min_profit_margin - recovery_relaxation).max(0.0)
    } else {
        min_profit_margin
    };

    let down_margin = if delta > 0.0 {
        // Need DOWN (heavy UP), relax DOWN margin
        (min_profit_margin - recovery_relaxation).max(0.0)
    } else {
        min_profit_margin
    };

    // For UP bids: max = 1.0 - down_avg_price - margin
    // When one-sided (have UP but no DOWN), no cap - we're not building a pair, just profit-taking
    let max_up_bid = if inventory.down_avg_price > 0.0 && inventory.down_size > 0.0 {
        // Have DOWN position: cap based on existing avg
        Some(1.0 - inventory.down_avg_price - up_margin)
    } else if inventory.up_size > 0.0 {
        // Have UP but NO DOWN: one-sided position, no cap needed
        // We're not building a pair, just holding UP for profit-taking
        None
    } else if let Some(down_ask) = down_orderbook.best_ask_price() {
        // No inventory on either side: use best DOWN ask as proxy for potential avg
        Some(1.0 - down_ask - up_margin)
    } else {
        // No data: use conservative 50/50 split
        Some(0.50 - up_margin)
    };

    // For DOWN bids: max = 1.0 - up_avg_price - margin
    // When one-sided (have DOWN but no UP), no cap - we're not building a pair, just profit-taking
    let max_down_bid = if inventory.up_avg_price > 0.0 && inventory.up_size > 0.0 {
        // Have UP position: cap based on existing avg
        Some(1.0 - inventory.up_avg_price - down_margin)
    } else if inventory.down_size > 0.0 {
        // Have DOWN but NO UP: one-sided position, no cap needed
        // We're not building a pair, just holding DOWN for profit-taking
        None
    } else if let Some(up_ask) = up_orderbook.best_ask_price() {
        // No inventory on either side: use best UP ask as proxy for potential avg
        Some(1.0 - up_ask - down_margin)
    } else {
        // No data: use conservative 50/50 split
        Some(0.50 - down_margin)
    };

    debug!(
        "[Profitability] max_bids: UP={:.4?}, DOWN={:.4?} (up_margin={:.3}, down_margin={:.3}, recovery_relax={:.3})",
        max_up_bid, max_down_bid, up_margin, down_margin, recovery_relaxation
    );

    (max_up_bid, max_down_bid)
}

/// Check if current inventory is already unprofitable.
/// Returns true if combined avg cost >= 1.0 - margin (unprofitable territory).
pub fn is_inventory_unprofitable(inventory: &InventorySnapshot, min_profit_margin: f64) -> bool {
    // Only check if we have both sides
    if inventory.up_size > 0.0 && inventory.down_size > 0.0 {
        let combined = inventory.combined_avg_cost();
        let max_cost = 1.0 - min_profit_margin;

        if combined >= max_cost {
            debug!(
                "[Profitability] Inventory unprofitable: combined={:.4} >= max={:.4}",
                combined, max_cost
            );
            return true;
        }
    }
    false
}

/// Calculate projected average price after a fill.
///
/// Returns the new weighted average if we add `fill_size` at `fill_price`.
pub fn projected_avg_after_fill(
    current_size: f64,
    current_avg: f64,
    fill_price: f64,
    fill_size: f64,
) -> f64 {
    if current_size < 1e-9 || current_avg < 1e-9 {
        // No existing position, new avg is just the fill price
        return fill_price;
    }

    let current_cost = current_size * current_avg;
    let new_cost = fill_size * fill_price;
    let new_total_size = current_size + fill_size;

    (current_cost + new_cost) / new_total_size
}

/// Check if a proposed fill would keep the position profitable.
///
/// Returns true if filling at `fill_price` would keep combined avg < 1.0 - margin.
pub fn would_fill_be_profitable(
    inventory: &InventorySnapshot,
    is_up_side: bool,
    fill_price: f64,
    fill_size: f64,
    min_profit_margin: f64,
) -> bool {
    let projected_up_avg = if is_up_side {
        projected_avg_after_fill(
            inventory.up_size,
            inventory.up_avg_price,
            fill_price,
            fill_size,
        )
    } else {
        if inventory.up_avg_price > 0.0 {
            inventory.up_avg_price
        } else {
            return true; // No UP position, can't check
        }
    };

    let projected_down_avg = if !is_up_side {
        projected_avg_after_fill(
            inventory.down_size,
            inventory.down_avg_price,
            fill_price,
            fill_size,
        )
    } else {
        if inventory.down_avg_price > 0.0 {
            inventory.down_avg_price
        } else {
            return true; // No DOWN position, can't check
        }
    };

    let projected_combined = projected_up_avg + projected_down_avg;
    let max_cost = 1.0 - min_profit_margin;

    projected_combined < max_cost
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_inventory(up_size: f64, up_avg: f64, down_size: f64, down_avg: f64) -> InventorySnapshot {
        InventorySnapshot {
            up_size,
            up_avg_price: up_avg,
            down_size,
            down_avg_price: down_avg,
        }
    }

    fn make_orderbook(ask: f64) -> OrderbookSnapshot {
        OrderbookSnapshot {
            best_ask: Some((ask, 100.0)),
            best_bid: Some((ask - 0.02, 50.0)),
            best_bid_is_ours: false,
            best_ask_is_ours: false,
        }
    }

    #[test]
    fn test_max_bids_with_inventory() {
        let inventory = make_inventory(50.0, 0.52, 50.0, 0.46);
        let up_ob = make_orderbook(0.55);
        let down_ob = make_orderbook(0.45);

        // No recovery (relaxation=0), balanced (delta=0)
        let (max_up, max_down) = calculate_max_bids(&inventory, &up_ob, &down_ob, 0.01, 0.0, 0.0);

        // max_up = 1.0 - 0.46 - 0.01 = 0.53
        assert!((max_up.unwrap() - 0.53).abs() < 0.001);
        // max_down = 1.0 - 0.52 - 0.01 = 0.47
        assert!((max_down.unwrap() - 0.47).abs() < 0.001);
    }

    #[test]
    fn test_max_bids_no_inventory() {
        let inventory = InventorySnapshot::default();
        let up_ob = make_orderbook(0.55);
        let down_ob = make_orderbook(0.45);

        // No recovery (relaxation=0), balanced (delta=0)
        let (max_up, max_down) = calculate_max_bids(&inventory, &up_ob, &down_ob, 0.01, 0.0, 0.0);

        // max_up = 1.0 - down_ask(0.45) - 0.01 = 0.54
        assert!((max_up.unwrap() - 0.54).abs() < 0.001);
        // max_down = 1.0 - up_ask(0.55) - 0.01 = 0.44
        assert!((max_down.unwrap() - 0.44).abs() < 0.001);
    }

    #[test]
    fn test_max_bids_recovery_relaxation() {
        let inventory = make_inventory(50.0, 0.52, 50.0, 0.46);
        let up_ob = make_orderbook(0.55);
        let down_ob = make_orderbook(0.45);

        // Heavy UP (delta=0.5), need DOWN - relax DOWN margin by 0.005
        let (max_up, max_down) = calculate_max_bids(&inventory, &up_ob, &down_ob, 0.01, 0.005, 0.5);

        // max_up uses full margin (not needed): 1.0 - 0.46 - 0.01 = 0.53
        assert!((max_up.unwrap() - 0.53).abs() < 0.001);
        // max_down uses relaxed margin: 1.0 - 0.52 - 0.005 = 0.475
        assert!((max_down.unwrap() - 0.475).abs() < 0.001);
    }

    #[test]
    fn test_check_recovery_status() {
        // Profitable: combined = 0.98 < 0.99
        let profitable = make_inventory(50.0, 0.52, 50.0, 0.46);
        let status = check_recovery_status(&profitable, 0.99, 1.02);
        assert!(!status.in_recovery);
        assert!(!status.is_stuck);

        // Recovery: combined = 1.00 >= 0.99
        let recovery = make_inventory(50.0, 0.52, 50.0, 0.48);
        let status = check_recovery_status(&recovery, 0.99, 1.02);
        assert!(status.in_recovery);
        assert!(!status.is_stuck);

        // Stuck: combined = 1.03 >= 1.02
        let stuck = make_inventory(50.0, 0.52, 50.0, 0.51);
        let status = check_recovery_status(&stuck, 0.99, 1.02);
        assert!(status.in_recovery);
        assert!(status.is_stuck);

        // One side only: not in recovery
        let one_side = make_inventory(50.0, 0.52, 0.0, 0.0);
        let status = check_recovery_status(&one_side, 0.99, 1.02);
        assert!(!status.in_recovery);
        assert!(!status.is_stuck);
    }

    #[test]
    fn test_is_inventory_unprofitable_profitable() {
        let inventory = make_inventory(50.0, 0.52, 50.0, 0.46);
        // Combined = 0.98, max = 0.99
        assert!(!is_inventory_unprofitable(&inventory, 0.01));
    }

    #[test]
    fn test_is_inventory_unprofitable_breakeven() {
        let inventory = make_inventory(50.0, 0.52, 50.0, 0.47);
        // Combined = 0.99, max = 0.99
        assert!(is_inventory_unprofitable(&inventory, 0.01));
    }

    #[test]
    fn test_is_inventory_unprofitable_loss() {
        let inventory = make_inventory(50.0, 0.52, 50.0, 0.49);
        // Combined = 1.01, max = 0.99
        assert!(is_inventory_unprofitable(&inventory, 0.01));
    }

    #[test]
    fn test_is_inventory_unprofitable_one_side_only() {
        // One side only - can't determine, so not considered unprofitable
        let inventory = make_inventory(50.0, 0.52, 0.0, 0.0);
        assert!(!is_inventory_unprofitable(&inventory, 0.01));
    }

    #[test]
    fn test_projected_avg_no_inventory() {
        let avg = projected_avg_after_fill(0.0, 0.0, 0.50, 100.0);
        assert!((avg - 0.50).abs() < 0.001);
    }

    #[test]
    fn test_projected_avg_with_inventory() {
        // 50 @ 0.52, add 100 @ 0.54 = (26 + 54) / 150 = 0.5333
        let avg = projected_avg_after_fill(50.0, 0.52, 0.54, 100.0);
        assert!((avg - 0.5333).abs() < 0.001);
    }

    #[test]
    fn test_would_fill_be_profitable_yes() {
        let inventory = make_inventory(50.0, 0.52, 50.0, 0.46);
        // Current combined = 0.98
        // Fill UP at 0.52, projected UP = ~0.52, combined = 0.98 < 0.99 - profitable
        assert!(would_fill_be_profitable(&inventory, true, 0.52, 100.0, 0.01));
    }

    #[test]
    fn test_would_fill_be_profitable_no() {
        let inventory = make_inventory(50.0, 0.52, 50.0, 0.46);
        // Fill UP at 0.60, projected UP avg goes up, combined > 0.99 - unprofitable
        // projected UP = (50*0.52 + 100*0.60) / 150 = (26 + 60) / 150 = 0.5733
        // combined = 0.5733 + 0.46 = 1.0333 > 0.99
        assert!(!would_fill_be_profitable(&inventory, true, 0.60, 100.0, 0.01));
    }

    #[test]
    fn test_max_bids_one_sided_up_only() {
        // One-sided: Have UP @ 0.445, no DOWN
        // This is the user's exact scenario
        let inventory = make_inventory(20.0, 0.445, 0.0, 0.0);
        let up_ob = make_orderbook(0.21);   // UP crashed
        let down_ob = make_orderbook(0.78); // DOWN rose

        let (max_up, max_down) = calculate_max_bids(&inventory, &up_ob, &down_ob, 0.01, 0.0, 1.0);

        // max_up should be None - we're not building a pair, just holding UP
        // This allows profit-taking without artificial cap
        assert!(max_up.is_none(), "One-sided UP should have no cap on UP bids");

        // max_down should use UP avg: 1.0 - 0.445 - 0.01 = 0.545
        // This is correct - buying DOWN above this would lock in losses
        assert!((max_down.unwrap() - 0.545).abs() < 0.001);
    }

    #[test]
    fn test_max_bids_one_sided_down_only() {
        // One-sided: Have DOWN @ 0.78, no UP
        let inventory = make_inventory(0.0, 0.0, 20.0, 0.78);
        let up_ob = make_orderbook(0.21);
        let down_ob = make_orderbook(0.78);

        let (max_up, max_down) = calculate_max_bids(&inventory, &up_ob, &down_ob, 0.01, 0.0, -1.0);

        // max_up should use DOWN avg: 1.0 - 0.78 - 0.01 = 0.21
        assert!((max_up.unwrap() - 0.21).abs() < 0.001);

        // max_down should be None - we're not building a pair, just holding DOWN
        assert!(max_down.is_none(), "One-sided DOWN should have no cap on DOWN bids");
    }
}
