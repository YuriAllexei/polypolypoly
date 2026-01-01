//! Property-based tests for the Inventory MM Solver
//!
//! Uses proptest to verify invariants that should hold for all inputs.
//!
//! Run with: cargo test -p polymarket solver_properties --release

mod common;

use proptest::prelude::*;

// ============================================================================
// Imbalance Calculation Properties
// ============================================================================

/// Calculate imbalance from inventory sizes
/// Formula: (up - down) / (up + down), returns 0 if both are 0
fn calculate_imbalance(up_size: f64, down_size: f64) -> f64 {
    let total = up_size + down_size;
    if total == 0.0 {
        0.0
    } else {
        (up_size - down_size) / total
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// Imbalance should always be in the range [-1, 1]
    #[test]
    fn imbalance_bounded(up in 0.0..10000.0f64, down in 0.0..10000.0f64) {
        let imbalance = calculate_imbalance(up, down);
        prop_assert!(imbalance >= -1.0, "Imbalance {} < -1", imbalance);
        prop_assert!(imbalance <= 1.0, "Imbalance {} > 1", imbalance);
    }

    /// Imbalance of balanced inventory should be 0
    #[test]
    fn imbalance_zero_when_balanced(size in 0.1..10000.0f64) {
        let imbalance = calculate_imbalance(size, size);
        prop_assert!((imbalance - 0.0).abs() < 1e-10, "Imbalance should be 0 for balanced, got {}", imbalance);
    }

    /// Imbalance should be 1 when only up inventory
    #[test]
    fn imbalance_one_when_only_up(up in 0.1..10000.0f64) {
        let imbalance = calculate_imbalance(up, 0.0);
        prop_assert!((imbalance - 1.0).abs() < 1e-10, "Imbalance should be 1, got {}", imbalance);
    }

    /// Imbalance should be -1 when only down inventory
    #[test]
    fn imbalance_neg_one_when_only_down(down in 0.1..10000.0f64) {
        let imbalance = calculate_imbalance(0.0, down);
        prop_assert!((imbalance - (-1.0)).abs() < 1e-10, "Imbalance should be -1, got {}", imbalance);
    }

    /// Imbalance should be symmetric: swap up/down, get negative
    #[test]
    fn imbalance_antisymmetric(up in 0.0..10000.0f64, down in 0.0..10000.0f64) {
        let imb1 = calculate_imbalance(up, down);
        let imb2 = calculate_imbalance(down, up);
        prop_assert!((imb1 + imb2).abs() < 1e-10, "Imbalance should be antisymmetric");
    }
}

// ============================================================================
// BPS Calculation Properties
// ============================================================================

/// Calculate BPS difference between two prices
fn calculate_bps(price_to_beat: f64, oracle_price: f64) -> f64 {
    if price_to_beat == 0.0 {
        return f64::INFINITY;
    }
    ((price_to_beat - oracle_price).abs() / price_to_beat) * 10000.0
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// BPS should always be non-negative
    #[test]
    fn bps_non_negative(
        price_to_beat in 0.01..100000.0f64,
        oracle_price in 0.01..100000.0f64
    ) {
        let bps = calculate_bps(price_to_beat, oracle_price);
        prop_assert!(bps >= 0.0, "BPS should be non-negative, got {}", bps);
    }

    /// BPS should be zero when prices are equal
    #[test]
    fn bps_zero_when_equal(price in 0.01..100000.0f64) {
        let bps = calculate_bps(price, price);
        prop_assert!(bps.abs() < 1e-10, "BPS should be 0 when prices equal, got {}", bps);
    }

    /// BPS should be symmetric (same distance regardless of direction)
    #[test]
    fn bps_symmetric_around_target(
        price_to_beat in 100.0..50000.0f64,
        diff_pct in 0.01..10.0f64
    ) {
        let above = price_to_beat * (1.0 + diff_pct / 100.0);
        let below = price_to_beat * (1.0 - diff_pct / 100.0);

        let bps_above = calculate_bps(price_to_beat, above);
        let bps_below = calculate_bps(price_to_beat, below);

        // Should be approximately equal (within 1%)
        let ratio = if bps_above > 0.0 { bps_below / bps_above } else { 1.0 };
        prop_assert!((ratio - 1.0).abs() < 0.01, "BPS should be symmetric: above={}, below={}", bps_above, bps_below);
    }

    /// 1% difference should be ~100 BPS
    #[test]
    fn bps_one_percent_is_100(price_to_beat in 100.0..50000.0f64) {
        let oracle_price = price_to_beat * 1.01; // 1% higher
        let bps = calculate_bps(price_to_beat, oracle_price);
        prop_assert!((bps - 100.0).abs() < 1.0, "1% should be ~100 BPS, got {}", bps);
    }
}

// ============================================================================
// Quote Price Properties
// ============================================================================

/// Calculate a hypothetical quote price with offset
fn calculate_quote_price(best_ask: f64, base_offset: f64, delta: f64, offset_scaling: f64) -> f64 {
    let offset = base_offset * (1.0 + delta.max(0.0) * offset_scaling);
    (best_ask - offset).max(0.01).min(0.99)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// Quote prices should always be positive
    #[test]
    fn quote_prices_positive(
        best_ask in 0.1..0.95f64,
        base_offset in 0.01..0.1f64,
        delta in -1.0..1.0f64,
        offset_scaling in 0.0..5.0f64
    ) {
        let price = calculate_quote_price(best_ask, base_offset, delta, offset_scaling);
        prop_assert!(price > 0.0, "Quote price should be positive, got {}", price);
    }

    /// Quote prices should be less than 1
    #[test]
    fn quote_prices_less_than_one(
        best_ask in 0.1..0.95f64,
        base_offset in 0.01..0.1f64,
        delta in -1.0..1.0f64,
        offset_scaling in 0.0..5.0f64
    ) {
        let price = calculate_quote_price(best_ask, base_offset, delta, offset_scaling);
        prop_assert!(price < 1.0, "Quote price should be < 1, got {}", price);
    }

    /// Quote prices should be below best ask (we're bidding)
    #[test]
    fn quote_prices_below_best_ask(
        best_ask in 0.2..0.9f64,
        base_offset in 0.01..0.05f64,
        delta in -1.0..1.0f64,
        offset_scaling in 0.0..2.0f64
    ) {
        let price = calculate_quote_price(best_ask, base_offset, delta, offset_scaling);
        prop_assert!(price <= best_ask, "Quote should be <= best_ask: {} > {}", price, best_ask);
    }

    /// Offset should increase with positive delta
    #[test]
    fn offset_increases_with_positive_delta(
        best_ask in 0.3..0.8f64,
        base_offset in 0.02..0.05f64,
        offset_scaling in 0.5..3.0f64
    ) {
        let price_neutral = calculate_quote_price(best_ask, base_offset, 0.0, offset_scaling);
        let price_positive = calculate_quote_price(best_ask, base_offset, 0.5, offset_scaling);

        // With positive delta, offset increases, so price decreases
        prop_assert!(
            price_positive <= price_neutral,
            "Price should decrease with positive delta: {} > {}",
            price_positive,
            price_neutral
        );
    }
}

// ============================================================================
// Skew Sizing Properties
// ============================================================================

/// Calculate skew-adjusted size
fn calculate_skewed_size(base_size: f64, delta: f64, skew_factor: f64) -> f64 {
    let skew = 1.0 + delta * skew_factor;
    let clamped_skew = skew.clamp(0.5, 2.0);
    (base_size * clamped_skew).max(0.0)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// Skewed sizes should always be non-negative
    #[test]
    fn skewed_size_non_negative(
        base_size in 0.0..1000.0f64,
        delta in -1.0..1.0f64,
        skew_factor in 0.0..5.0f64
    ) {
        let size = calculate_skewed_size(base_size, delta, skew_factor);
        prop_assert!(size >= 0.0, "Skewed size should be non-negative, got {}", size);
    }

    /// Skew multiplier should be bounded between 0.5 and 2.0
    #[test]
    fn skew_multiplier_bounded(
        delta in -1.0..1.0f64,
        skew_factor in 0.0..10.0f64
    ) {
        let skew = 1.0 + delta * skew_factor;
        let clamped = skew.clamp(0.5, 2.0);
        prop_assert!(clamped >= 0.5, "Clamped skew should be >= 0.5");
        prop_assert!(clamped <= 2.0, "Clamped skew should be <= 2.0");
    }

    /// Zero delta should give base size
    #[test]
    fn zero_delta_gives_base_size(
        base_size in 0.0..1000.0f64,
        skew_factor in 0.0..5.0f64
    ) {
        let size = calculate_skewed_size(base_size, 0.0, skew_factor);
        prop_assert!((size - base_size).abs() < 1e-10, "Zero delta should give base size");
    }
}

// ============================================================================
// Order Diff Properties
// ============================================================================

/// Simulate order diff logic - returns (to_cancel, to_add) counts
fn simulate_diff(
    current_prices: &[f64],
    desired_prices: &[f64],
    price_tolerance: f64,
) -> (usize, usize) {
    let mut to_cancel = 0;
    let mut to_add = 0;

    // Check which current orders need to be cancelled
    for &current in current_prices {
        let has_match = desired_prices
            .iter()
            .any(|&desired| (current - desired).abs() <= price_tolerance);
        if !has_match {
            to_cancel += 1;
        }
    }

    // Check which desired orders need to be added
    for &desired in desired_prices {
        let has_match = current_prices
            .iter()
            .any(|&current| (current - desired).abs() <= price_tolerance);
        if !has_match {
            to_add += 1;
        }
    }

    (to_cancel, to_add)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    /// Diffing identical orders should produce no changes
    #[test]
    fn diff_identical_is_empty(
        price1 in 0.1..0.9f64,
        price2 in 0.1..0.9f64,
        price3 in 0.1..0.9f64
    ) {
        let orders = vec![price1, price2, price3];
        let (cancel, add) = simulate_diff(&orders, &orders, 0.0001);
        prop_assert_eq!(cancel, 0, "Should have no cancels");
        prop_assert_eq!(add, 0, "Should have no adds");
    }

    /// Diffing with empty current should add all desired
    #[test]
    fn diff_empty_current_adds_all(
        price1 in 0.1..0.9f64,
        price2 in 0.1..0.9f64
    ) {
        let current: Vec<f64> = vec![];
        let desired = vec![price1, price2];
        let (cancel, add) = simulate_diff(&current, &desired, 0.0001);
        prop_assert_eq!(cancel, 0, "Should have no cancels");
        prop_assert_eq!(add, 2, "Should add all desired");
    }

    /// Diffing with empty desired should cancel all current
    #[test]
    fn diff_empty_desired_cancels_all(
        price1 in 0.1..0.9f64,
        price2 in 0.1..0.9f64
    ) {
        let current = vec![price1, price2];
        let desired: Vec<f64> = vec![];
        let (cancel, add) = simulate_diff(&current, &desired, 0.0001);
        prop_assert_eq!(cancel, 2, "Should cancel all current");
        prop_assert_eq!(add, 0, "Should have no adds");
    }
}

// ============================================================================
// EIP-712 Signing Properties (Simplified)
// ============================================================================

/// Simulate domain separator computation (deterministic)
fn compute_domain_hash(chain_id: u64, contract_suffix: u8) -> u64 {
    // Simplified: just combine values deterministically
    chain_id.wrapping_mul(1000).wrapping_add(contract_suffix as u64)
}

/// Simulate struct hash computation (deterministic)
fn compute_struct_hash(salt: u64, amount: u64, price: u64) -> u64 {
    // Simplified: combine values deterministically
    salt.wrapping_mul(17)
        .wrapping_add(amount.wrapping_mul(31))
        .wrapping_add(price.wrapping_mul(37))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    /// Domain separator should be deterministic
    #[test]
    fn domain_separator_deterministic(chain_id in 1u64..1000, suffix in 0u8..10) {
        let hash1 = compute_domain_hash(chain_id, suffix);
        let hash2 = compute_domain_hash(chain_id, suffix);
        prop_assert_eq!(hash1, hash2, "Domain separator should be deterministic");
    }

    /// Struct hash should be deterministic
    #[test]
    fn struct_hash_deterministic(salt in 0u64..u64::MAX, amount in 0u64..1000000, price in 0u64..1000000) {
        let hash1 = compute_struct_hash(salt, amount, price);
        let hash2 = compute_struct_hash(salt, amount, price);
        prop_assert_eq!(hash1, hash2, "Struct hash should be deterministic");
    }

    /// Different inputs should (usually) produce different hashes
    #[test]
    fn different_inputs_different_hash(
        salt1 in 0u64..u64::MAX,
        salt2 in 0u64..u64::MAX,
        amount in 0u64..1000000,
        price in 0u64..1000000
    ) {
        prop_assume!(salt1 != salt2);
        let hash1 = compute_struct_hash(salt1, amount, price);
        let hash2 = compute_struct_hash(salt2, amount, price);
        // Note: collisions possible but unlikely
        prop_assert_ne!(hash1, hash2, "Different salts should produce different hashes");
    }
}
