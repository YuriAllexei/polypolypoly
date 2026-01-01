//! Benchmarks for HyperSockets library
//!
//! Run with: cargo bench -p hypersockets

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use std::sync::Arc;
use std::time::Duration;

// Re-export types from the library
use hypersockets::core::connection_state::{AtomicConnectionState, AtomicMetrics, ConnectionState};
use hypersockets::core::pong_tracker::PongTracker;
use hypersockets::traits::reconnect::{ExponentialBackoff, FixedDelay, ReconnectionStrategy};

/// Benchmark atomic state operations
fn bench_atomic_state(c: &mut Criterion) {
    let mut group = c.benchmark_group("atomic_state");

    // Benchmark state get
    group.bench_function("get", |b| {
        let state = AtomicConnectionState::new(ConnectionState::Connected);
        b.iter(|| black_box(state.get()))
    });

    // Benchmark state set
    group.bench_function("set", |b| {
        let state = AtomicConnectionState::new(ConnectionState::Disconnected);
        b.iter(|| {
            state.set(black_box(ConnectionState::Connected));
        })
    });

    // Benchmark compare_exchange success
    group.bench_function("compare_exchange_success", |b| {
        let state = AtomicConnectionState::new(ConnectionState::Disconnected);
        b.iter(|| {
            let _ = state.compare_exchange(
                black_box(ConnectionState::Disconnected),
                black_box(ConnectionState::Connecting),
            );
            state.set(ConnectionState::Disconnected); // Reset for next iteration
        })
    });

    // Benchmark compare_exchange failure
    group.bench_function("compare_exchange_failure", |b| {
        let state = AtomicConnectionState::new(ConnectionState::Connected);
        b.iter(|| {
            let _ = state.compare_exchange(
                black_box(ConnectionState::Disconnected),
                black_box(ConnectionState::Connecting),
            );
        })
    });

    // Benchmark is_connected check
    group.bench_function("is_connected", |b| {
        let state = AtomicConnectionState::new(ConnectionState::Connected);
        b.iter(|| black_box(state.is_connected()))
    });

    group.finish();
}

/// Benchmark atomic metrics operations
fn bench_atomic_metrics(c: &mut Criterion) {
    let mut group = c.benchmark_group("atomic_metrics");

    // Benchmark increment_sent
    group.bench_function("increment_sent", |b| {
        let metrics = AtomicMetrics::new();
        b.iter(|| {
            metrics.increment_sent();
        })
    });

    // Benchmark increment_received
    group.bench_function("increment_received", |b| {
        let metrics = AtomicMetrics::new();
        b.iter(|| {
            metrics.increment_received();
        })
    });

    // Benchmark get messages_sent
    group.bench_function("messages_sent", |b| {
        let metrics = AtomicMetrics::new();
        metrics.increment_sent();
        b.iter(|| black_box(metrics.messages_sent()))
    });

    // Benchmark reset
    group.bench_function("reset", |b| {
        let metrics = AtomicMetrics::new();
        metrics.increment_sent();
        metrics.increment_received();
        b.iter(|| {
            metrics.reset();
        })
    });

    group.finish();
}

/// Benchmark pong tracker operations
fn bench_pong_tracker(c: &mut Criterion) {
    let mut group = c.benchmark_group("pong_tracker");

    // Benchmark record_ping_sent
    group.bench_function("record_ping_sent", |b| {
        let tracker = PongTracker::new(Duration::from_secs(15));
        b.iter(|| {
            tracker.record_ping_sent();
        })
    });

    // Benchmark record_pong_received
    group.bench_function("record_pong_received", |b| {
        let tracker = PongTracker::new(Duration::from_secs(15));
        b.iter(|| {
            tracker.record_pong_received();
        })
    });

    // Benchmark is_healthy (no ping sent yet)
    group.bench_function("is_healthy_no_ping", |b| {
        let tracker = PongTracker::new(Duration::from_secs(15));
        b.iter(|| black_box(tracker.is_healthy()))
    });

    // Benchmark is_healthy (ping sent, pong received)
    group.bench_function("is_healthy_after_pong", |b| {
        let tracker = PongTracker::new(Duration::from_secs(15));
        tracker.record_ping_sent();
        tracker.record_pong_received();
        b.iter(|| black_box(tracker.is_healthy()))
    });

    // Benchmark is_healthy (ping sent, no pong yet)
    group.bench_function("is_healthy_awaiting_pong", |b| {
        let tracker = PongTracker::new(Duration::from_secs(15));
        tracker.record_ping_sent();
        b.iter(|| black_box(tracker.is_healthy()))
    });

    group.finish();
}

/// Benchmark reconnection strategy calculations
fn bench_reconnection_strategies(c: &mut Criterion) {
    let mut group = c.benchmark_group("reconnection_strategies");

    // Benchmark ExponentialBackoff next_delay
    group.bench_function("exponential_backoff_next_delay", |b| {
        let strategy = ExponentialBackoff::new(
            Duration::from_millis(100),
            Duration::from_secs(30),
            None,
        );
        b.iter(|| black_box(strategy.next_delay(black_box(5))))
    });

    // Benchmark FixedDelay next_delay
    group.bench_function("fixed_delay_next_delay", |b| {
        let strategy = FixedDelay::new(Duration::from_millis(500), None);
        b.iter(|| black_box(strategy.next_delay(black_box(5))))
    });

    // Benchmark should_reconnect
    group.bench_function("exponential_should_reconnect", |b| {
        let strategy = ExponentialBackoff::new(
            Duration::from_millis(100),
            Duration::from_secs(30),
            Some(10),
        );
        b.iter(|| black_box(strategy.should_reconnect(black_box(5))))
    });

    group.finish();
}

/// Benchmark concurrent access patterns
fn bench_concurrent_access(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrent_access");
    group.throughput(Throughput::Elements(1));

    // Benchmark Arc-wrapped state access
    group.bench_function("arc_state_get", |b| {
        let state = Arc::new(AtomicConnectionState::new(ConnectionState::Connected));
        b.iter(|| black_box(state.get()))
    });

    // Benchmark Arc-wrapped metrics increment
    group.bench_function("arc_metrics_increment", |b| {
        let metrics = Arc::new(AtomicMetrics::new());
        b.iter(|| {
            metrics.increment_sent();
        })
    });

    group.finish();
}

/// Benchmark channel throughput simulation
fn bench_channel_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("channel_throughput");
    group.throughput(Throughput::Elements(1));

    // Benchmark crossbeam unbounded channel send/recv
    group.bench_function("crossbeam_unbounded_send", |b| {
        let (tx, rx) = crossbeam_channel::unbounded::<u64>();
        b.iter(|| {
            tx.send(black_box(42)).unwrap();
            rx.recv().unwrap();
        })
    });

    // Benchmark crossbeam bounded channel send/recv
    group.bench_function("crossbeam_bounded_send", |b| {
        let (tx, rx) = crossbeam_channel::bounded::<u64>(100);
        b.iter(|| {
            tx.send(black_box(42)).unwrap();
            rx.recv().unwrap();
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_atomic_state,
    bench_atomic_metrics,
    bench_pong_tracker,
    bench_reconnection_strategies,
    bench_concurrent_access,
    bench_channel_throughput,
);

criterion_main!(benches);
