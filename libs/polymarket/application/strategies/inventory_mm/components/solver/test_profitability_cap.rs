//! Comprehensive tests for profitability cap behavior.
//!
//! These tests expose the interaction between profitability cap and rebalancing,
//! helping to fine-tune the weighted bid approach.

use crate::application::strategies::inventory_mm::types::{
    InventorySnapshot, OrderbookSnapshot, SolverConfig,
};

use super::quotes::{calculate_quotes, calculate_generated_bid, round_to_tick};

fn default_config() -> SolverConfig {
    SolverConfig {
        num_levels: 3,
        tick_size: 0.01,
        base_offset: 0.01,
        max_imbalance: 0.8,
        order_size: 100.0,
        spread_per_level: 1.0,
        offset_scaling: 5.0,
        skew_factor: 1.0,
        min_offset: 0.01,
        max_position: 0.0,
        prof_weight: 0.3,
        imbalance_weight: 0.7,
        prof_cap_delta_threshold: 0.15,  // Only apply cap when |delta| <= 0.15
    }
}

/// Scenario result for detailed analysis
struct ScenarioResult {
    // Inputs
    down_avg_price: f64,
    up_best_bid: f64,
    up_best_ask: f64,
    delta: f64,
    prof_weight: f64,
    imbalance_weight: f64,
    // Calculated
    prof_bid: f64,
    generated_bid: f64,
    // Outputs
    up_quote_prices: Vec<f64>,
    levels_from_best_bid: f64,
}

fn run_scenario(
    down_avg_price: f64,
    up_best_bid: f64,
    up_best_ask: f64,
    delta: f64,
    prof_weight: f64,
    imbalance_weight: f64,
) -> ScenarioResult {
    run_scenario_with_threshold(down_avg_price, up_best_bid, up_best_ask, delta, prof_weight, imbalance_weight, 0.3)
}

fn run_scenario_with_threshold(
    down_avg_price: f64,
    up_best_bid: f64,
    up_best_ask: f64,
    delta: f64,
    prof_weight: f64,
    imbalance_weight: f64,
    prof_cap_delta_threshold: f64,
) -> ScenarioResult {
    let mut config = default_config();
    config.prof_weight = prof_weight;
    config.imbalance_weight = imbalance_weight;
    config.prof_cap_delta_threshold = prof_cap_delta_threshold;

    let inventory = InventorySnapshot {
        up_size: 0.0,
        up_avg_price: 0.0,
        down_size: 20.0,
        down_avg_price,
    };

    let up_ob = OrderbookSnapshot {
        best_ask: Some((up_best_ask, 100.0)),
        best_bid: Some((up_best_bid, 50.0)),
        best_bid_is_ours: false,
        best_ask_is_ours: false,
    };
    let down_ob = OrderbookSnapshot {
        best_ask: Some((1.0 - up_best_bid - 0.02, 100.0)),
        best_bid: Some((1.0 - up_best_ask - 0.02, 50.0)),
        best_bid_is_ours: false,
        best_ask_is_ours: false,
    };

    let ladder = calculate_quotes(delta, &up_ob, &down_ob, &inventory, &config, "up", "down");

    let prof_bid = 1.0 - down_avg_price;
    let generated_bid = calculate_generated_bid(prof_bid, up_best_bid, prof_weight, imbalance_weight);

    let up_quote_prices: Vec<f64> = ladder.up_quotes.iter().map(|q| q.price).collect();
    let top_bid = up_quote_prices.first().copied().unwrap_or(0.0);
    let levels_from_best_bid = (up_best_bid - top_bid) / 0.01;

    ScenarioResult {
        down_avg_price,
        up_best_bid,
        up_best_ask,
        delta,
        prof_weight,
        imbalance_weight,
        prof_bid,
        generated_bid,
        up_quote_prices,
        levels_from_best_bid,
    }
}

// =============================================================================
// CORE PROBLEM: Rebalancing scenario
// =============================================================================

#[test]
fn test_user_exact_rebalancing_scenario() {
    // EXACT USER SCENARIO:
    // - Got 20 DOWN shares filled at $0.57
    // - Need to quote aggressively on UP to cover imbalance
    // - With the fix, profitability cap is DISABLED when |delta| > threshold
    //   allowing aggressive rebalancing via offset/skew mechanisms

    println!("\n=== USER'S EXACT REBALANCING SCENARIO (FIXED) ===\n");

    let scenario = run_scenario(
        0.57,  // DOWN avg price
        0.50,  // UP best_bid
        0.52,  // UP best_ask
        -1.0,  // delta = -1.0 (100% DOWN, need UP!)
        0.3,   // prof_weight
        0.7,   // imbalance_weight
    );

    println!("Inventory: 20 DOWN @ ${:.2}", scenario.down_avg_price);
    println!("UP market: bid=${:.2}, ask=${:.2}", scenario.up_best_bid, scenario.up_best_ask);
    println!("Delta: {:.2} (need UP to rebalance)", scenario.delta);
    println!("prof_cap_delta_threshold: 0.3 (cap disabled when |delta| > 0.3)");
    println!();
    println!("Result:");
    println!("  UP quote prices: {:?}", scenario.up_quote_prices);
    println!("  Levels from best_bid: {:.0}", scenario.levels_from_best_bid);
    println!();

    // With the FIX:
    // |delta| = 1.0 > threshold (0.3), so profitability cap is DISABLED
    // Offset mechanism takes over: offset = min_offset = 0.01
    // Price = best_ask - offset = 0.52 - 0.01 = 0.51
    // We should be very close to best_bid (within 1-2 levels)

    assert!(
        scenario.levels_from_best_bid <= 2.0,
        "FIX VERIFIED: When rebalancing (|delta| > threshold), we should be <=2 levels from best_bid, got {:.0}",
        scenario.levels_from_best_bid
    );
}

#[test]
fn test_cap_applied_when_balanced() {
    // When balanced (|delta| <= threshold), cap SHOULD be applied
    println!("\n=== CAP APPLIED WHEN BALANCED ===\n");

    let scenario = run_scenario(
        0.57,  // DOWN avg price
        0.50,  // UP best_bid
        0.52,  // UP best_ask
        0.0,   // delta = 0.0 (balanced!)
        0.3,   // prof_weight
        0.7,   // imbalance_weight
    );

    println!("Delta: {:.2} (balanced, cap should apply)", scenario.delta);
    println!("UP quote prices: {:?}", scenario.up_quote_prices);
    println!("Levels from best_bid: {:.0}", scenario.levels_from_best_bid);

    // With balanced delta, cap IS applied
    // generated_bid = (0.43 * 0.3 + 0.50 * 0.7) = 0.47
    // Should be 3 levels from best_bid
    assert!(
        scenario.levels_from_best_bid >= 2.0,
        "When balanced, cap should be applied, expected >=2 levels from best_bid, got {:.0}",
        scenario.levels_from_best_bid
    );
}

#[test]
fn test_weight_sensitivity() {
    println!("\n=== WEIGHT SENSITIVITY ANALYSIS ===\n");
    println!("{:<10} {:<10} {:<12} {:<12} {:<10}",
        "prof_w", "imbal_w", "prof_bid", "gen_bid", "levels");
    println!("{}", "-".repeat(55));

    let weights = [
        (0.0, 1.0, "Pure market"),
        (0.1, 0.9, "10% profit"),
        (0.2, 0.8, "20% profit"),
        (0.3, 0.7, "30% profit (default)"),
        (0.5, 0.5, "50/50"),
        (0.7, 0.3, "70% profit"),
        (1.0, 0.0, "Pure profit"),
    ];

    for (pw, iw, _label) in weights {
        let s = run_scenario(0.57, 0.50, 0.52, -0.5, pw, iw);
        println!("{:<10.1} {:<10.1} ${:<11.2} ${:<11.2} {:<10.0}",
            pw, iw, s.prof_bid, s.generated_bid, s.levels_from_best_bid);
    }

    // With pure market weight (0, 1), should be competitive
    let pure_market = run_scenario(0.57, 0.50, 0.52, -0.5, 0.0, 1.0);
    assert!(
        pure_market.levels_from_best_bid <= 2.0,
        "Pure market weight should be <=2 levels away, got {:.0}",
        pure_market.levels_from_best_bid
    );
}

#[test]
fn test_avg_cost_sensitivity() {
    // With the new threshold-based cap, test at delta=0.0 (within threshold)
    // to see how avg cost affects the cap when it IS applied
    println!("\n=== DOWN AVG COST SENSITIVITY (at delta=0) ===\n");
    println!("{:<12} {:<10} {:<10} {:<12}",
        "down_avg", "prof_bid", "gen_bid", "levels");
    println!("{}", "-".repeat(45));

    let avgs = [0.40, 0.45, 0.50, 0.55, 0.57, 0.60, 0.65, 0.70];

    for avg in avgs {
        // Use delta=0.0 so cap is applied (within threshold)
        let s = run_scenario(avg, 0.50, 0.52, 0.0, 0.3, 0.7);
        println!("${:<11.2} ${:<9.2} ${:<9.2} {:<12.0}",
            avg, s.prof_bid, s.generated_bid, s.levels_from_best_bid);
    }

    // When cap IS applied (delta=0.0), higher avg cost = more restrictive cap
    let low_cost = run_scenario(0.40, 0.50, 0.52, 0.0, 0.3, 0.7);
    let high_cost = run_scenario(0.70, 0.50, 0.52, 0.0, 0.3, 0.7);

    assert!(
        high_cost.levels_from_best_bid > low_cost.levels_from_best_bid,
        "When cap is applied (delta=0), higher avg cost should be more restrictive"
    );
}

#[test]
fn test_offset_vs_cap_interaction() {
    // When delta is negative (need UP), offset decreases (aggressive)
    // But profitability cap may override this

    println!("\n=== OFFSET VS CAP INTERACTION ===\n");

    let config = default_config();
    let up_best_ask = 0.52;
    let down_avg = 0.57;
    let prof_bid = 1.0 - down_avg;
    let up_best_bid = 0.50;

    println!("{:<8} {:<12} {:<12} {:<12} {:<12}",
        "delta", "offset", "offset_px", "cap_px", "final");
    println!("{}", "-".repeat(60));

    for delta in [-1.0, -0.5, 0.0, 0.5, 1.0] {
        let offset = (config.base_offset * (1.0 + delta * config.offset_scaling))
            .max(config.min_offset);
        let offset_price = round_to_tick(up_best_ask - offset, 0.01);
        let cap_price = calculate_generated_bid(prof_bid, up_best_bid, 0.3, 0.7);
        let final_price = offset_price.min(cap_price);

        println!("{:<8.1} {:<12.3} ${:<11.2} ${:<11.2} ${:<11.2}",
            delta, offset, offset_price, cap_price, final_price);
    }

    println!();
    println!("PROBLEM: When delta=-1.0 (need UP urgently):");
    println!("  offset = min_offset = $0.01 (very aggressive)");
    println!("  offset_price = $0.51 (close to ask!)");
    println!("  BUT cap_price = $0.47");
    println!("  final = min($0.51, $0.47) = $0.47 (CAP WINS!)");
}

#[test]
fn test_cap_disabled_vs_enabled() {
    // Test cap behavior at delta=0.0 (within threshold, so cap IS applied)
    println!("\n=== CAP DISABLED VS ENABLED (at delta=0) ===\n");

    // At delta=0.0 (within threshold), cap is applied when prof_weight > 0
    let disabled = run_scenario(0.57, 0.50, 0.52, 0.0, 0.0, 1.0);
    let enabled = run_scenario(0.57, 0.50, 0.52, 0.0, 0.3, 0.7);

    println!("Cap DISABLED (prof_weight=0):");
    println!("  UP quotes: {:?}", disabled.up_quote_prices);
    println!("  Levels from best_bid: {:.0}", disabled.levels_from_best_bid);
    println!();
    println!("Cap ENABLED (prof_weight=0.3, delta=0 within threshold):");
    println!("  UP quotes: {:?}", enabled.up_quote_prices);
    println!("  Levels from best_bid: {:.0}", enabled.levels_from_best_bid);

    assert!(
        disabled.levels_from_best_bid < enabled.levels_from_best_bid,
        "Disabled cap should be more competitive than enabled cap (at delta=0)"
    );
}

#[test]
fn test_cap_bypassed_when_imbalanced() {
    // When delta > threshold, cap is bypassed regardless of prof_weight
    println!("\n=== CAP BYPASSED WHEN IMBALANCED ===\n");

    // At delta=-1.0 (outside threshold), cap is bypassed
    let with_prof_weight = run_scenario(0.57, 0.50, 0.52, -1.0, 0.3, 0.7);
    let without_prof_weight = run_scenario(0.57, 0.50, 0.52, -1.0, 0.0, 1.0);

    println!("With prof_weight=0.3 (but delta=-1.0, so cap bypassed):");
    println!("  UP quotes: {:?}", with_prof_weight.up_quote_prices);
    println!("  Levels from best_bid: {:.0}", with_prof_weight.levels_from_best_bid);
    println!();
    println!("With prof_weight=0.0:");
    println!("  UP quotes: {:?}", without_prof_weight.up_quote_prices);
    println!("  Levels from best_bid: {:.0}", without_prof_weight.levels_from_best_bid);

    // Both should be equally competitive since cap is bypassed
    assert!(
        (with_prof_weight.levels_from_best_bid - without_prof_weight.levels_from_best_bid).abs() < 0.5,
        "When imbalanced (|delta| > threshold), cap should be bypassed regardless of prof_weight"
    );
}

// =============================================================================
// EDGE CASES
// =============================================================================

#[test]
fn test_already_profitable_position() {
    // If combined avg < 1.0, we're profitable
    // Cap should be less restrictive

    println!("\n=== ALREADY PROFITABLE POSITION ===\n");

    let config = default_config();

    // UP avg = 0.40, DOWN avg = 0.40 → combined = 0.80 < 1.0 ✓
    let inventory = InventorySnapshot {
        up_size: 20.0,
        up_avg_price: 0.40,
        down_size: 20.0,
        down_avg_price: 0.40,
    };

    let up_ob = OrderbookSnapshot {
        best_ask: Some((0.52, 100.0)),
        best_bid: Some((0.50, 50.0)),
        best_bid_is_ours: false,
        best_ask_is_ours: false,
    };
    let down_ob = OrderbookSnapshot {
        best_ask: Some((0.50, 100.0)),
        best_bid: Some((0.48, 50.0)),
        best_bid_is_ours: false,
        best_ask_is_ours: false,
    };

    let ladder = calculate_quotes(0.0, &up_ob, &down_ob, &inventory, &config, "up", "down");

    println!("Position: UP=${:.2}, DOWN=${:.2}, combined=${:.2}",
        inventory.up_avg_price, inventory.down_avg_price,
        inventory.up_avg_price + inventory.down_avg_price);
    println!("prof_bid_UP = 1.0 - {:.2} = {:.2}", inventory.down_avg_price, 1.0 - inventory.down_avg_price);
    println!("UP quotes: {:?}", ladder.up_quotes.iter().map(|q| q.price).collect::<Vec<_>>());

    // prof_bid = 0.60 > best_bid (0.50)
    // generated_bid = (0.60 * 0.3 + 0.50 * 0.7) = 0.53
    // This is > offset price (0.51), so cap doesn't restrict
    assert!(!ladder.up_quotes.is_empty());
    assert!(
        (ladder.up_quotes[0].price - 0.51).abs() < 0.02,
        "Should be at offset price, not restricted by cap"
    );
}

#[test]
fn test_underwater_position() {
    // If combined avg > 1.0, we're underwater
    // Cap will be very restrictive

    println!("\n=== UNDERWATER POSITION (LOSING) ===\n");

    let config = default_config();

    // UP avg = 0.60, DOWN avg = 0.55 → combined = 1.15 > 1.0 ✗
    let inventory = InventorySnapshot {
        up_size: 20.0,
        up_avg_price: 0.60,
        down_size: 20.0,
        down_avg_price: 0.55,
    };

    let up_ob = OrderbookSnapshot {
        best_ask: Some((0.52, 100.0)),
        best_bid: Some((0.50, 50.0)),
        best_bid_is_ours: false,
        best_ask_is_ours: false,
    };
    let down_ob = OrderbookSnapshot {
        best_ask: Some((0.50, 100.0)),
        best_bid: Some((0.48, 50.0)),
        best_bid_is_ours: false,
        best_ask_is_ours: false,
    };

    let ladder = calculate_quotes(0.0, &up_ob, &down_ob, &inventory, &config, "up", "down");

    println!("Position: UP=${:.2}, DOWN=${:.2}, combined=${:.2} (UNDERWATER!)",
        inventory.up_avg_price, inventory.down_avg_price,
        inventory.up_avg_price + inventory.down_avg_price);
    println!("prof_bid_UP = 1.0 - {:.2} = {:.2}", inventory.down_avg_price, 1.0 - inventory.down_avg_price);
    println!("prof_bid_DOWN = 1.0 - {:.2} = {:.2}", inventory.up_avg_price, 1.0 - inventory.up_avg_price);
    println!("UP quotes: {:?}", ladder.up_quotes.iter().map(|q| q.price).collect::<Vec<_>>());
    println!("DOWN quotes: {:?}", ladder.down_quotes.iter().map(|q| q.price).collect::<Vec<_>>());

    // QUESTION: Should we even apply cap when already underwater?
    // The cap will be very restrictive and may prevent us from recovering.
}

#[test]
fn test_symmetric_behavior() {
    // Cap should work symmetrically for UP and DOWN

    println!("\n=== SYMMETRY TEST ===\n");

    let config = default_config();

    // Scenario A: Heavy DOWN, need UP
    let inv_a = InventorySnapshot {
        up_size: 0.0,
        up_avg_price: 0.0,
        down_size: 20.0,
        down_avg_price: 0.55,
    };

    // Scenario B: Heavy UP, need DOWN
    let inv_b = InventorySnapshot {
        up_size: 20.0,
        up_avg_price: 0.55,
        down_size: 0.0,
        down_avg_price: 0.0,
    };

    let up_ob = OrderbookSnapshot {
        best_ask: Some((0.52, 100.0)),
        best_bid: Some((0.50, 50.0)),
        best_bid_is_ours: false,
        best_ask_is_ours: false,
    };
    let down_ob = OrderbookSnapshot {
        best_ask: Some((0.50, 100.0)),
        best_bid: Some((0.48, 50.0)),
        best_bid_is_ours: false,
        best_ask_is_ours: false,
    };

    let ladder_a = calculate_quotes(-0.5, &up_ob, &down_ob, &inv_a, &config, "up", "down");
    let ladder_b = calculate_quotes(0.5, &up_ob, &down_ob, &inv_b, &config, "up", "down");

    println!("Scenario A (need UP): {:?}",
        ladder_a.up_quotes.iter().map(|q| q.price).collect::<Vec<_>>());
    println!("Scenario B (need DOWN): {:?}",
        ladder_b.down_quotes.iter().map(|q| q.price).collect::<Vec<_>>());

    assert!(!ladder_a.up_quotes.is_empty(), "Should quote UP");
    assert!(!ladder_b.down_quotes.is_empty(), "Should quote DOWN");
}

// =============================================================================
// PROPOSED FIX CONCEPTS
// =============================================================================

#[test]
fn test_concept_dynamic_weights() {
    // IDEA: Reduce prof_weight when we need to rebalance urgently

    println!("\n=== CONCEPT: DYNAMIC WEIGHTS ===\n");
    println!("Reduce prof_weight based on |delta| to allow aggressive rebalancing\n");

    let base_prof_weight = 0.3;

    println!("{:<8} {:<12} {:<12} {:<10}",
        "delta", "adj_prof_w", "gen_bid", "levels");
    println!("{}", "-".repeat(45));

    for delta in [-1.0_f64, -0.5, 0.0, 0.5, 1.0] {
        // Reduce prof_weight based on urgency
        let urgency = delta.abs();
        let adj_prof_w = base_prof_weight * (1.0 - urgency * 0.8);
        let adj_imbal_w = 1.0 - adj_prof_w;

        let s = run_scenario(0.57, 0.50, 0.52, delta, adj_prof_w, adj_imbal_w);

        println!("{:<8.1} {:<12.2} ${:<11.2} {:<10.0}",
            delta, adj_prof_w, s.generated_bid, s.levels_from_best_bid);
    }

    println!();
    println!("With dynamic weights:");
    println!("  delta=-1.0 → prof_w≈0.06, almost pure market pricing");
    println!("  delta=0.0  → prof_w=0.30, normal profitability check");
}

#[test]
fn test_concept_cap_only_when_balanced() {
    // IDEA: Only apply cap when we're relatively balanced
    // When imbalanced, disable cap to allow rebalancing

    println!("\n=== CONCEPT: CAP ONLY WHEN BALANCED ===\n");

    let threshold = 0.3;  // Don't apply cap when |delta| > threshold

    println!("{:<8} {:<12} {:<12} {:<10}",
        "delta", "apply_cap", "gen_bid", "levels");
    println!("{}", "-".repeat(45));

    for delta in [-1.0_f64, -0.5, -0.3, 0.0, 0.3, 0.5, 1.0] {
        let apply_cap = delta.abs() <= threshold;
        let pw = if apply_cap { 0.3 } else { 0.0 };
        let iw = if apply_cap { 0.7 } else { 1.0 };

        let s = run_scenario(0.57, 0.50, 0.52, delta, pw, iw);

        println!("{:<8.1} {:<12} ${:<11.2} {:<10.0}",
            delta, apply_cap, s.generated_bid, s.levels_from_best_bid);
    }

    println!();
    println!("With balanced-only cap:");
    println!("  |delta| > 0.3 → no cap, aggressive rebalancing");
    println!("  |delta| <= 0.3 → cap applied, maintain profitability");
}

#[test]
fn test_concept_soft_cap() {
    // IDEA: Use cap as a soft limit that influences but doesn't hard-cap

    println!("\n=== CONCEPT: SOFT CAP ===\n");
    println!("Instead of hard cap, blend between offset price and cap price\n");

    let config = default_config();
    let up_best_ask = 0.52;
    let down_avg = 0.57;
    let up_best_bid = 0.50;
    let prof_bid = 1.0 - down_avg;  // 0.43
    let cap_price = calculate_generated_bid(prof_bid, up_best_bid, 0.3, 0.7);  // 0.47

    println!("{:<8} {:<12} {:<12} {:<12} {:<12}",
        "delta", "offset_px", "cap_px", "blend_0.5", "hard_cap");
    println!("{}", "-".repeat(60));

    for delta in [-1.0, -0.5, 0.0, 0.5, 1.0] {
        let offset = (config.base_offset * (1.0 + delta * config.offset_scaling))
            .max(config.min_offset);
        let offset_price = round_to_tick(up_best_ask - offset, 0.01);

        // Soft cap: blend between offset and cap
        let blend_factor = 0.5;
        let soft_price = offset_price * (1.0 - blend_factor) + cap_price * blend_factor;

        // Hard cap
        let hard_price = offset_price.min(cap_price);

        println!("{:<8.1} ${:<11.2} ${:<11.2} ${:<11.2} ${:<11.2}",
            delta, offset_price, cap_price, soft_price, hard_price);
    }

    println!();
    println!("Soft cap allows more competitive pricing while still");
    println!("pulling toward profitability.");
}
