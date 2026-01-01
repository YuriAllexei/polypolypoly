//! Benchmarks for the Inventory MM Solver
//!
//! Run with: cargo bench -p polymarket --bench solver_bench

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

// ============================================================================
// Imbalance Calculation Benchmark
// ============================================================================

/// Calculate imbalance from inventory sizes
fn calculate_imbalance(up_size: f64, down_size: f64) -> f64 {
    let total = up_size + down_size;
    if total == 0.0 {
        0.0
    } else {
        (up_size - down_size) / total
    }
}

fn bench_imbalance_calculation(c: &mut Criterion) {
    let mut group = c.benchmark_group("imbalance");

    // Benchmark with different inventory sizes
    for (up, down) in [(50.0, 50.0), (100.0, 0.0), (1000.0, 500.0), (10000.0, 9000.0)] {
        group.bench_with_input(
            BenchmarkId::new("calculate", format!("{}_{}", up, down)),
            &(up, down),
            |b, &(up, down)| {
                b.iter(|| calculate_imbalance(black_box(up), black_box(down)))
            },
        );
    }

    group.finish();
}

// ============================================================================
// BPS Calculation Benchmark
// ============================================================================

/// Calculate BPS difference between two prices
fn calculate_bps(price_to_beat: f64, oracle_price: f64) -> f64 {
    ((price_to_beat - oracle_price).abs() / price_to_beat) * 10000.0
}

fn bench_bps_calculation(c: &mut Criterion) {
    let mut group = c.benchmark_group("bps");

    let test_cases = [
        (50000.0, 50500.0), // 1% diff
        (50000.0, 50050.0), // 0.1% diff
        (50000.0, 50005.0), // 0.01% diff
        (1000.0, 1010.0),   // Small price 1% diff
    ];

    for (target, oracle) in test_cases {
        group.bench_with_input(
            BenchmarkId::new("calculate", format!("{:.0}_{:.0}", target, oracle)),
            &(target, oracle),
            |b, &(target, oracle)| {
                b.iter(|| calculate_bps(black_box(target), black_box(oracle)))
            },
        );
    }

    group.finish();
}

// ============================================================================
// Quote Price Calculation Benchmark
// ============================================================================

/// Calculate quote price with offset
fn calculate_quote_price(best_ask: f64, base_offset: f64, delta: f64, offset_scaling: f64) -> f64 {
    let offset = base_offset * (1.0 + delta.max(0.0) * offset_scaling);
    (best_ask - offset).max(0.01).min(0.99)
}

fn bench_quote_calculation(c: &mut Criterion) {
    let mut group = c.benchmark_group("quote");

    let configs = [
        (0.55, 0.02, 0.0, 1.0),   // Neutral delta
        (0.55, 0.02, 0.5, 1.0),   // Positive delta
        (0.55, 0.02, -0.5, 1.0),  // Negative delta
        (0.55, 0.02, 0.8, 2.0),   // High delta, high scaling
    ];

    for (best_ask, offset, delta, scaling) in configs {
        group.bench_with_input(
            BenchmarkId::new("calculate", format!("d{:.1}_s{:.1}", delta, scaling)),
            &(best_ask, offset, delta, scaling),
            |b, &(best_ask, offset, delta, scaling)| {
                b.iter(|| {
                    calculate_quote_price(
                        black_box(best_ask),
                        black_box(offset),
                        black_box(delta),
                        black_box(scaling),
                    )
                })
            },
        );
    }

    group.finish();
}

// ============================================================================
// Skew Sizing Benchmark
// ============================================================================

/// Calculate skew-adjusted size
fn calculate_skewed_size(base_size: f64, delta: f64, skew_factor: f64) -> f64 {
    let skew = 1.0 + delta * skew_factor;
    let clamped_skew = skew.clamp(0.5, 2.0);
    (base_size * clamped_skew).max(0.0)
}

fn bench_skew_sizing(c: &mut Criterion) {
    let mut group = c.benchmark_group("skew");

    let configs = [
        (100.0, 0.0, 1.0),   // No skew
        (100.0, 0.5, 1.0),   // Moderate skew
        (100.0, 0.8, 2.0),   // High skew
        (100.0, -0.8, 2.0),  // Negative skew
    ];

    for (size, delta, factor) in configs {
        group.bench_with_input(
            BenchmarkId::new("calculate", format!("d{:.1}_f{:.1}", delta, factor)),
            &(size, delta, factor),
            |b, &(size, delta, factor)| {
                b.iter(|| {
                    calculate_skewed_size(
                        black_box(size),
                        black_box(delta),
                        black_box(factor),
                    )
                })
            },
        );
    }

    group.finish();
}

// ============================================================================
// Quote Ladder Generation Benchmark
// ============================================================================

/// Simulate generating a quote ladder with multiple levels
fn generate_quote_ladder(
    best_ask: f64,
    base_offset: f64,
    level_spacing: f64,
    num_levels: usize,
    delta: f64,
    offset_scaling: f64,
) -> Vec<(f64, f64)> {
    let mut quotes = Vec::with_capacity(num_levels);
    let base_size = 100.0;

    for i in 0..num_levels {
        let level_offset = base_offset + (i as f64) * level_spacing;
        let price = calculate_quote_price(best_ask, level_offset, delta, offset_scaling);
        let size = calculate_skewed_size(base_size, delta, 1.0);
        quotes.push((price, size));
    }

    quotes
}

fn bench_quote_ladder(c: &mut Criterion) {
    let mut group = c.benchmark_group("quote_ladder");

    for num_levels in [3, 5, 10, 20] {
        group.bench_with_input(
            BenchmarkId::new("generate", num_levels),
            &num_levels,
            |b, &num_levels| {
                b.iter(|| {
                    generate_quote_ladder(
                        black_box(0.55),
                        black_box(0.02),
                        black_box(0.01),
                        black_box(num_levels),
                        black_box(0.3),
                        black_box(1.0),
                    )
                })
            },
        );
    }

    group.finish();
}

// ============================================================================
// Full Solve Simulation Benchmark
// ============================================================================

/// Simulate a full solve cycle
fn simulate_solve(
    up_size: f64,
    down_size: f64,
    up_ask: f64,
    down_ask: f64,
    num_levels: usize,
) -> (Vec<(f64, f64)>, Vec<(f64, f64)>) {
    let delta = calculate_imbalance(up_size, down_size);

    let up_quotes = generate_quote_ladder(up_ask, 0.02, 0.01, num_levels, delta, 1.0);
    let down_quotes = generate_quote_ladder(down_ask, 0.02, 0.01, num_levels, -delta, 1.0);

    (up_quotes, down_quotes)
}

fn bench_full_solve(c: &mut Criterion) {
    let mut group = c.benchmark_group("full_solve");

    let test_cases = [
        ("balanced", 50.0, 50.0, 0.55, 0.45, 3),
        ("imbalanced", 80.0, 20.0, 0.55, 0.45, 3),
        ("deep", 50.0, 50.0, 0.55, 0.45, 10),
        ("very_deep", 50.0, 50.0, 0.55, 0.45, 20),
    ];

    for (name, up, down, up_ask, down_ask, levels) in test_cases {
        group.bench_with_input(
            BenchmarkId::new("solve", name),
            &(up, down, up_ask, down_ask, levels),
            |b, &(up, down, up_ask, down_ask, levels)| {
                b.iter(|| {
                    simulate_solve(
                        black_box(up),
                        black_box(down),
                        black_box(up_ask),
                        black_box(down_ask),
                        black_box(levels),
                    )
                })
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_imbalance_calculation,
    bench_bps_calculation,
    bench_quote_calculation,
    bench_skew_sizing,
    bench_quote_ladder,
    bench_full_solve,
);

criterion_main!(benches);
