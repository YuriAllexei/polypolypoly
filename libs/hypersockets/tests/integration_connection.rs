//! Integration tests for WebSocket connection management
//!
//! These tests verify connection state transitions and lifecycle management.

mod common;

use hypersockets::core::connection_state::{AtomicConnectionState, AtomicMetrics, ConnectionState};
use std::sync::Arc;
use std::thread;

/// Macro for verbose test output
macro_rules! verbose_println {
    ($($arg:tt)*) => {
        if std::env::var("TEST_VERBOSE").is_ok() {
            println!($($arg)*);
        }
    };
}

#[test]
fn test_connection_state_full_lifecycle() {
    verbose_println!("Testing full connection lifecycle...");

    let state = AtomicConnectionState::new(ConnectionState::Disconnected);

    // Initial state
    assert!(state.is_disconnected());
    verbose_println!("  Initial state: Disconnected");

    // Connect
    state.set(ConnectionState::Connecting);
    assert!(state.is_connecting());
    verbose_println!("  State: Connecting");

    // Connected
    state.set(ConnectionState::Connected);
    assert!(state.is_connected());
    verbose_println!("  State: Connected");

    // Disconnect
    state.set(ConnectionState::ShuttingDown);
    assert!(state.is_shutting_down());
    verbose_println!("  State: ShuttingDown");

    state.set(ConnectionState::Disconnected);
    assert!(state.is_disconnected());
    verbose_println!("  State: Disconnected (complete)");
}

#[test]
fn test_connection_state_reconnection_cycle() {
    verbose_println!("Testing reconnection cycle...");

    let state = AtomicConnectionState::new(ConnectionState::Connected);
    let metrics = AtomicMetrics::new();

    // Simulate disconnection and reconnection
    for i in 0..3 {
        verbose_println!("  Reconnection attempt {}", i + 1);

        // Lost connection
        state.set(ConnectionState::Reconnecting);
        assert!(state.is_connecting()); // is_connecting includes Reconnecting

        // Increment reconnect counter
        metrics.increment_reconnects();

        // Reconnected
        state.set(ConnectionState::Connected);
        assert!(state.is_connected());
    }

    assert_eq!(metrics.reconnect_count(), 3);
    verbose_println!("  Total reconnections: {}", metrics.reconnect_count());
}

#[test]
fn test_concurrent_state_access() {
    verbose_println!("Testing concurrent state access...");

    let state = Arc::new(AtomicConnectionState::new(ConnectionState::Disconnected));
    let metrics = Arc::new(AtomicMetrics::new());

    let mut handles = vec![];

    // Spawn readers
    for _ in 0..5 {
        let state_clone = Arc::clone(&state);
        handles.push(thread::spawn(move || {
            for _ in 0..1000 {
                let _ = state_clone.get();
                let _ = state_clone.is_connected();
            }
        }));
    }

    // Spawn writers
    for _ in 0..3 {
        let state_clone = Arc::clone(&state);
        handles.push(thread::spawn(move || {
            for _ in 0..100 {
                state_clone.set(ConnectionState::Connected);
                state_clone.set(ConnectionState::Disconnected);
            }
        }));
    }

    // Spawn metrics updaters
    for _ in 0..5 {
        let metrics_clone = Arc::clone(&metrics);
        handles.push(thread::spawn(move || {
            for _ in 0..1000 {
                metrics_clone.increment_sent();
                metrics_clone.increment_received();
            }
        }));
    }

    for handle in handles {
        handle.join().unwrap();
    }

    // Verify metrics consistency
    assert_eq!(metrics.messages_sent(), 5000);
    assert_eq!(metrics.messages_received(), 5000);
    verbose_println!("  Concurrent access completed successfully");
}

#[test]
fn test_compare_exchange_race_safety() {
    verbose_println!("Testing compare_exchange race safety...");

    let state = Arc::new(AtomicConnectionState::new(ConnectionState::Disconnected));
    let success_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let mut handles = vec![];

    // Multiple threads try to be the first to transition
    for _ in 0..10 {
        let state_clone = Arc::clone(&state);
        let success_clone = Arc::clone(&success_count);

        handles.push(thread::spawn(move || {
            if state_clone
                .compare_exchange(ConnectionState::Disconnected, ConnectionState::Connecting)
                .is_ok()
            {
                success_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }));
    }

    for handle in handles {
        handle.join().unwrap();
    }

    // Only one thread should have succeeded
    assert_eq!(
        success_count.load(std::sync::atomic::Ordering::Relaxed),
        1,
        "Only one thread should win the race"
    );
    verbose_println!("  Race safety verified: exactly 1 winner");
}

#[test]
fn test_metrics_under_high_load() {
    verbose_println!("Testing metrics under high load...");

    let metrics = Arc::new(AtomicMetrics::new());
    let num_threads = 20;
    let ops_per_thread = 10_000;

    let mut handles = vec![];

    for _ in 0..num_threads {
        let metrics_clone = Arc::clone(&metrics);
        handles.push(thread::spawn(move || {
            for _ in 0..ops_per_thread {
                metrics_clone.increment_sent();
            }
        }));
    }

    for handle in handles {
        handle.join().unwrap();
    }

    let expected = (num_threads * ops_per_thread) as u64;
    assert_eq!(metrics.messages_sent(), expected);
    verbose_println!(
        "  High load test passed: {} operations",
        num_threads * ops_per_thread
    );
}

#[test]
fn test_state_transitions_with_metrics() {
    verbose_println!("Testing state transitions with metrics tracking...");

    let state = AtomicConnectionState::new(ConnectionState::Disconnected);
    let metrics = AtomicMetrics::new();

    // Simulate connection with message exchange
    state.set(ConnectionState::Connected);

    for _ in 0..10 {
        metrics.increment_sent();
        metrics.increment_received();
    }

    // Disconnect
    state.set(ConnectionState::Disconnected);

    assert_eq!(metrics.messages_sent(), 10);
    assert_eq!(metrics.messages_received(), 10);
    assert_eq!(metrics.reconnect_count(), 0);

    // Reconnect
    metrics.increment_reconnects();
    state.set(ConnectionState::Connected);

    assert_eq!(metrics.reconnect_count(), 1);
    verbose_println!("  State transitions with metrics: OK");
}
