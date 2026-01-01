//! Benchmarks for Order Diff Algorithm
//!
//! Run with: cargo bench -p polymarket --bench order_diff_bench

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use std::collections::HashSet;

// ============================================================================
// Order Diff Algorithm
// ============================================================================

/// Represents an order at a price level
#[derive(Clone, Debug)]
struct Order {
    id: String,
    price: f64,
    size: f64,
}

/// Represents a desired quote
#[derive(Clone, Debug)]
struct Quote {
    price: f64,
    size: f64,
}

/// Result of diffing current orders against desired quotes
#[derive(Debug, Default)]
struct DiffResult {
    to_cancel: Vec<String>,
    to_place: Vec<Quote>,
    unchanged: usize,
}

/// Price tolerance for matching orders to quotes (0.1%)
const PRICE_TOLERANCE: f64 = 0.001;
/// Size tolerance for matching (1%)
const SIZE_TOLERANCE: f64 = 0.01;

/// Check if two prices are within tolerance
fn prices_match(p1: f64, p2: f64) -> bool {
    (p1 - p2).abs() <= p1 * PRICE_TOLERANCE
}

/// Check if two sizes are within tolerance
fn sizes_match(s1: f64, s2: f64) -> bool {
    if s1 == 0.0 && s2 == 0.0 {
        return true;
    }
    let max = s1.max(s2);
    (s1 - s2).abs() / max <= SIZE_TOLERANCE
}

/// Diff current orders against desired quotes
fn diff_orders(current: &[Order], desired: &[Quote]) -> DiffResult {
    let mut result = DiffResult::default();
    let mut matched_quote_indices: HashSet<usize> = HashSet::new();

    // For each current order, try to find a matching quote
    for order in current {
        let mut found_match = false;

        for (i, quote) in desired.iter().enumerate() {
            if matched_quote_indices.contains(&i) {
                continue;
            }

            if prices_match(order.price, quote.price) && sizes_match(order.size, quote.size) {
                matched_quote_indices.insert(i);
                result.unchanged += 1;
                found_match = true;
                break;
            }
        }

        if !found_match {
            result.to_cancel.push(order.id.clone());
        }
    }

    // Quotes not matched need to be placed
    for (i, quote) in desired.iter().enumerate() {
        if !matched_quote_indices.contains(&i) {
            result.to_place.push(quote.clone());
        }
    }

    result
}

// ============================================================================
// Test Data Generation
// ============================================================================

/// Generate a list of current orders
fn generate_orders(count: usize, base_price: f64, spacing: f64) -> Vec<Order> {
    (0..count)
        .map(|i| Order {
            id: format!("order-{}", i),
            price: base_price - (i as f64) * spacing,
            size: 100.0,
        })
        .collect()
}

/// Generate a list of desired quotes (similar to orders with slight differences)
fn generate_quotes_similar(count: usize, base_price: f64, spacing: f64) -> Vec<Quote> {
    (0..count)
        .map(|i| Quote {
            price: base_price - (i as f64) * spacing,
            size: 100.0,
        })
        .collect()
}

/// Generate quotes that are completely different
fn generate_quotes_different(count: usize, base_price: f64, spacing: f64) -> Vec<Quote> {
    (0..count)
        .map(|i| Quote {
            price: base_price - (i as f64) * spacing - 0.05, // Shifted
            size: 150.0, // Different size
        })
        .collect()
}

/// Generate quotes with some overlap
fn generate_quotes_partial(count: usize, base_price: f64, spacing: f64) -> Vec<Quote> {
    (0..count)
        .map(|i| Quote {
            price: if i % 2 == 0 {
                base_price - (i as f64) * spacing // Same
            } else {
                base_price - (i as f64) * spacing - 0.02 // Different
            },
            size: if i % 2 == 0 { 100.0 } else { 120.0 },
        })
        .collect()
}

// ============================================================================
// Benchmarks
// ============================================================================

fn bench_diff_no_changes(c: &mut Criterion) {
    let mut group = c.benchmark_group("diff_no_changes");

    for count in [3, 5, 10, 25, 50] {
        let orders = generate_orders(count, 0.55, 0.01);
        let quotes = generate_quotes_similar(count, 0.55, 0.01);

        group.bench_with_input(BenchmarkId::new("count", count), &(orders, quotes), |b, (orders, quotes)| {
            b.iter(|| diff_orders(black_box(orders), black_box(quotes)))
        });
    }

    group.finish();
}

fn bench_diff_all_changes(c: &mut Criterion) {
    let mut group = c.benchmark_group("diff_all_changes");

    for count in [3, 5, 10, 25, 50] {
        let orders = generate_orders(count, 0.55, 0.01);
        let quotes = generate_quotes_different(count, 0.55, 0.01);

        group.bench_with_input(BenchmarkId::new("count", count), &(orders, quotes), |b, (orders, quotes)| {
            b.iter(|| diff_orders(black_box(orders), black_box(quotes)))
        });
    }

    group.finish();
}

fn bench_diff_partial_changes(c: &mut Criterion) {
    let mut group = c.benchmark_group("diff_partial_changes");

    for count in [3, 5, 10, 25, 50] {
        let orders = generate_orders(count, 0.55, 0.01);
        let quotes = generate_quotes_partial(count, 0.55, 0.01);

        group.bench_with_input(BenchmarkId::new("count", count), &(orders, quotes), |b, (orders, quotes)| {
            b.iter(|| diff_orders(black_box(orders), black_box(quotes)))
        });
    }

    group.finish();
}

fn bench_diff_empty_to_full(c: &mut Criterion) {
    let mut group = c.benchmark_group("diff_empty_to_full");

    for count in [3, 5, 10, 25, 50] {
        let orders: Vec<Order> = vec![];
        let quotes = generate_quotes_similar(count, 0.55, 0.01);

        group.bench_with_input(BenchmarkId::new("count", count), &(orders, quotes), |b, (orders, quotes)| {
            b.iter(|| diff_orders(black_box(orders), black_box(quotes)))
        });
    }

    group.finish();
}

fn bench_diff_full_to_empty(c: &mut Criterion) {
    let mut group = c.benchmark_group("diff_full_to_empty");

    for count in [3, 5, 10, 25, 50] {
        let orders = generate_orders(count, 0.55, 0.01);
        let quotes: Vec<Quote> = vec![];

        group.bench_with_input(BenchmarkId::new("count", count), &(orders, quotes), |b, (orders, quotes)| {
            b.iter(|| diff_orders(black_box(orders), black_box(quotes)))
        });
    }

    group.finish();
}

fn bench_price_matching(c: &mut Criterion) {
    let mut group = c.benchmark_group("price_matching");

    let test_cases = [
        ("exact", 0.55, 0.55),
        ("within_tolerance", 0.55, 0.55005),
        ("outside_tolerance", 0.55, 0.56),
    ];

    for (name, p1, p2) in test_cases {
        group.bench_with_input(BenchmarkId::new("match", name), &(p1, p2), |b, &(p1, p2)| {
            b.iter(|| prices_match(black_box(p1), black_box(p2)))
        });
    }

    group.finish();
}

fn bench_size_matching(c: &mut Criterion) {
    let mut group = c.benchmark_group("size_matching");

    let test_cases = [
        ("exact", 100.0, 100.0),
        ("within_tolerance", 100.0, 100.5),
        ("outside_tolerance", 100.0, 110.0),
    ];

    for (name, s1, s2) in test_cases {
        group.bench_with_input(BenchmarkId::new("match", name), &(s1, s2), |b, &(s1, s2)| {
            b.iter(|| sizes_match(black_box(s1), black_box(s2)))
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_diff_no_changes,
    bench_diff_all_changes,
    bench_diff_partial_changes,
    bench_diff_empty_to_full,
    bench_diff_full_to_empty,
    bench_price_matching,
    bench_size_matching,
);

criterion_main!(benches);
