//! Common test utilities for Polymarket integration tests
//!
//! This module provides shared utilities, fixtures, and helpers for testing.

/// Macro for verbose test output (controlled by TEST_VERBOSE env var)
#[macro_export]
macro_rules! verbose_println {
    ($($arg:tt)*) => {
        if std::env::var("TEST_VERBOSE").is_ok() {
            println!($($arg)*);
        }
    };
}

/// Check if required environment variables are set for integration tests
pub fn has_api_credentials() -> bool {
    std::env::var("PRIVATE_KEY").is_ok()
        && std::env::var("API_KEY").is_ok()
        && std::env::var("API_SECRET").is_ok()
        && std::env::var("API_PASSPHRASE").is_ok()
}

/// Skip test if credentials are not available
#[macro_export]
macro_rules! skip_if_no_credentials {
    () => {
        if !$crate::common::has_api_credentials() {
            println!("Skipping test: API credentials not available");
            return;
        }
    };
}

pub mod fixtures {
    //! Test fixtures for common data types

    /// Create a test orderbook snapshot
    pub fn balanced_orderbook() -> (f64, f64, f64, f64) {
        // (best_bid_price, best_bid_size, best_ask_price, best_ask_size)
        (0.48, 500.0, 0.52, 500.0)
    }

    /// Create a tight spread orderbook
    pub fn tight_spread_orderbook() -> (f64, f64, f64, f64) {
        (0.50, 1000.0, 0.51, 1000.0)
    }

    /// Create an imbalanced inventory snapshot
    pub fn heavy_up_inventory() -> (f64, f64) {
        // (up_size, down_size)
        (80.0, 20.0)
    }

    /// Create a balanced inventory snapshot
    pub fn balanced_inventory() -> (f64, f64) {
        (50.0, 50.0)
    }

    /// Create empty inventory
    pub fn empty_inventory() -> (f64, f64) {
        (0.0, 0.0)
    }
}
