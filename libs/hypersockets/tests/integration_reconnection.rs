//! Integration tests for reconnection strategies
//!
//! These tests verify reconnection behavior with different strategies.

use hypersockets::traits::reconnect::{
    ExponentialBackoff, FixedDelay, NeverReconnect, ReconnectionStrategy,
};
use std::time::Duration;

/// Macro for verbose test output
macro_rules! verbose_println {
    ($($arg:tt)*) => {
        if std::env::var("TEST_VERBOSE").is_ok() {
            println!($($arg)*);
        }
    };
}

#[test]
fn test_exponential_backoff_full_sequence() {
    verbose_println!("Testing exponential backoff full sequence...");

    let strategy = ExponentialBackoff::new(
        Duration::from_millis(100),
        Duration::from_secs(10),
        Some(5),
    );

    let expected_delays = [100, 200, 400, 800, 1600];

    for (attempt, &expected_ms) in expected_delays.iter().enumerate() {
        let delay = strategy.next_delay(attempt).unwrap();
        verbose_println!("  Attempt {}: {:?}", attempt, delay);
        assert_eq!(
            delay.as_millis(),
            expected_ms,
            "Unexpected delay at attempt {}",
            attempt
        );
    }

    // Attempt 5 should return None (max_attempts = 5)
    assert!(
        strategy.next_delay(5).is_none(),
        "Should return None after max attempts"
    );
}

#[test]
fn test_exponential_backoff_with_capping() {
    verbose_println!("Testing exponential backoff with capping...");

    let strategy = ExponentialBackoff::new(
        Duration::from_millis(500),
        Duration::from_secs(2), // Cap at 2 seconds
        None,
    );

    // Attempt 0: 500ms
    // Attempt 1: 1000ms
    // Attempt 2: 2000ms (capped)
    // Attempt 3: 4000ms -> capped to 2000ms
    // etc.

    let delays: Vec<u64> = (0..6)
        .map(|i| strategy.next_delay(i).unwrap().as_millis() as u64)
        .collect();

    verbose_println!("  Delays: {:?}", delays);

    assert_eq!(delays[0], 500);
    assert_eq!(delays[1], 1000);
    assert_eq!(delays[2], 2000);
    assert_eq!(delays[3], 2000); // Capped
    assert_eq!(delays[4], 2000); // Capped
    assert_eq!(delays[5], 2000); // Capped
}

#[test]
fn test_fixed_delay_consistency() {
    verbose_println!("Testing fixed delay consistency...");

    let strategy = FixedDelay::new(Duration::from_millis(750), None);

    for attempt in 0..100 {
        let delay = strategy.next_delay(attempt).unwrap();
        assert_eq!(
            delay,
            Duration::from_millis(750),
            "Fixed delay should be constant"
        );
    }

    verbose_println!("  All 100 attempts returned 750ms");
}

#[test]
fn test_fixed_delay_with_max_attempts() {
    verbose_println!("Testing fixed delay with max attempts...");

    let strategy = FixedDelay::new(Duration::from_millis(500), Some(3));

    assert!(strategy.next_delay(0).is_some());
    assert!(strategy.next_delay(1).is_some());
    assert!(strategy.next_delay(2).is_some());
    assert!(strategy.next_delay(3).is_none()); // 4th attempt (0-indexed)

    verbose_println!("  Max attempts limit working correctly");
}

#[test]
fn test_never_reconnect_always_fails() {
    verbose_println!("Testing NeverReconnect strategy...");

    let strategy = NeverReconnect;

    for attempt in 0..10 {
        assert!(
            strategy.next_delay(attempt).is_none(),
            "NeverReconnect should always return None"
        );
        assert!(
            !strategy.should_reconnect(attempt),
            "NeverReconnect should never allow reconnection"
        );
    }

    verbose_println!("  NeverReconnect correctly prevents all reconnections");
}

#[test]
fn test_strategy_reset_behavior() {
    verbose_println!("Testing strategy reset behavior...");

    let mut exp = ExponentialBackoff::new(
        Duration::from_millis(100),
        Duration::from_secs(30),
        None,
    );
    let mut fixed = FixedDelay::new(Duration::from_millis(500), None);
    let mut never = NeverReconnect;

    // Record state before reset
    let exp_before = exp.next_delay(5);
    let fixed_before = fixed.next_delay(5);

    // Reset all
    exp.reset();
    fixed.reset();
    never.reset();

    // Verify state unchanged (these are stateless strategies)
    assert_eq!(exp.next_delay(5), exp_before);
    assert_eq!(fixed.next_delay(5), fixed_before);

    verbose_println!("  Reset behavior verified for all strategies");
}

#[test]
fn test_exponential_backoff_overflow_safety() {
    verbose_println!("Testing exponential backoff overflow safety...");

    let strategy = ExponentialBackoff::new(
        Duration::from_millis(100),
        Duration::from_secs(3600), // 1 hour max
        None,
    );

    // Test with very high attempt numbers
    // 100ms * 2^30 would overflow, but should be capped
    let delay = strategy.next_delay(30).unwrap();
    verbose_println!("  Delay at attempt 30: {:?}", delay);

    // Should be capped at max (1 hour)
    assert!(delay <= Duration::from_secs(3600));

    // Even at extreme values, should not panic
    let _ = strategy.next_delay(100);
    let _ = strategy.next_delay(1000);

    verbose_println!("  Overflow safety verified");
}

#[test]
fn test_mixed_strategy_simulation() {
    verbose_println!("Testing mixed strategy simulation...");

    // Simulate switching strategies based on error type
    fn get_delay_for_error(error_type: &str, attempt: usize) -> Option<Duration> {
        match error_type {
            "network" => {
                // Use exponential backoff for network errors
                let strategy = ExponentialBackoff::new(
                    Duration::from_millis(100),
                    Duration::from_secs(30),
                    Some(10),
                );
                strategy.next_delay(attempt)
            }
            "server_busy" => {
                // Use fixed delay for server busy
                let strategy = FixedDelay::new(Duration::from_secs(5), Some(3));
                strategy.next_delay(attempt)
            }
            "auth_failed" => {
                // Never retry auth failures
                let strategy = NeverReconnect;
                strategy.next_delay(attempt)
            }
            _ => None,
        }
    }

    // Network errors use exponential backoff
    assert_eq!(
        get_delay_for_error("network", 0),
        Some(Duration::from_millis(100))
    );
    assert_eq!(
        get_delay_for_error("network", 2),
        Some(Duration::from_millis(400))
    );

    // Server busy uses fixed delay
    assert_eq!(
        get_delay_for_error("server_busy", 0),
        Some(Duration::from_secs(5))
    );
    assert_eq!(
        get_delay_for_error("server_busy", 2),
        Some(Duration::from_secs(5))
    );

    // Auth failed never retries
    assert_eq!(get_delay_for_error("auth_failed", 0), None);

    verbose_println!("  Mixed strategy simulation successful");
}
