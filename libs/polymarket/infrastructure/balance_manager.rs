//! Balance Manager
//!
//! Monitors account balance with high watermark tracking.
//! Halts trading when balance drops below a configurable threshold of the peak.

use crate::infrastructure::client::clob::TradingClient;
use parking_lot::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

/// Manages balance monitoring with high watermark tracking
///
/// The manager tracks two balance values:
/// - `balance_pivot`: The high watermark (only increases)
/// - `balance_current`: The latest balance value
///
/// Trading is halted when current balance drops below the configured
/// threshold percentage of the pivot.
pub struct BalanceManager {
    balance_pivot: Arc<RwLock<f64>>,
    balance_current: Arc<RwLock<f64>>,
    halt_for_trading_balance: Arc<AtomicBool>,
    halt_threshold: f64,
    task_handle: Option<JoinHandle<()>>,
}

impl BalanceManager {
    /// Create a new balance manager with specified halt threshold
    ///
    /// # Arguments
    /// * `halt_threshold` - Fraction of pivot below which to halt (e.g., 0.10 for 10%)
    pub fn new(halt_threshold: f64) -> Self {
        Self {
            balance_pivot: Arc::new(RwLock::new(0.0)),
            balance_current: Arc::new(RwLock::new(0.0)),
            halt_for_trading_balance: Arc::new(AtomicBool::new(false)),
            halt_threshold,
            task_handle: None,
        }
    }

    /// Start the balance monitoring task
    ///
    /// Fetches initial balance, sets pivot and current to that value,
    /// then spawns a background task that polls every 1 second.
    pub async fn start(
        &mut self,
        trading: Arc<TradingClient>,
        shutdown_flag: Arc<AtomicBool>,
    ) -> anyhow::Result<()> {
        // Fetch initial balance
        let initial = trading.get_usd_balance().await?;
        *self.balance_pivot.write() = initial;
        *self.balance_current.write() = initial;

        let threshold_pct = self.halt_threshold * 100.0;
        info!(
            "BalanceManager started: initial balance ${:.2}, pivot=${:.2}, halt threshold={:.0}%",
            initial, initial, threshold_pct
        );

        // Clone values for the spawned task
        let pivot: Arc<RwLock<f64>> = Arc::clone(&self.balance_pivot);
        let current: Arc<RwLock<f64>> = Arc::clone(&self.balance_current);
        let halt = Arc::clone(&self.halt_for_trading_balance);
        let halt_threshold = self.halt_threshold;

        let handle = tokio::spawn(async move {
            while shutdown_flag.load(Ordering::Acquire) {
                tokio::time::sleep(Duration::from_secs(1)).await;

                // Check shutdown again after sleep
                if !shutdown_flag.load(Ordering::Acquire) {
                    break;
                }

                match trading.get_usd_balance().await {
                    Ok(new_balance) => {
                        // Scope for holding locks - extract values before async operations
                        let (should_halt, should_resume, pivot_val, threshold) = {
                            let mut pivot_guard = pivot.write();
                            let mut current_guard = current.write();

                            // Update pivot if new balance is higher (high watermark)
                            if new_balance > *pivot_guard {
                                info!(
                                    "BalanceManager: New high watermark ${:.2} (was ${:.2})",
                                    new_balance, *pivot_guard
                                );
                                *pivot_guard = new_balance;
                            }

                            // Always update current balance
                            *current_guard = new_balance;

                            // Calculate threshold based on configured percentage
                            let threshold = *pivot_guard * halt_threshold;
                            let was_halted = halt.load(Ordering::Acquire);

                            let should_halt = new_balance < threshold && !was_halted;
                            let should_resume = new_balance >= threshold && was_halted;
                            let pivot_val = *pivot_guard;

                            (should_halt, should_resume, pivot_val, threshold)
                        }; // Guards dropped here

                        if should_halt {
                            // Balance dropped below threshold - halt trading
                            warn!(
                                "BalanceManager: HALTING - balance ${:.2} < {:.0}% of pivot ${:.2} (threshold: ${:.2})",
                                new_balance, halt_threshold * 100.0, pivot_val, threshold
                            );
                            halt.store(true, Ordering::Release);

                            // Cancel all open orders when halting
                            match trading.cancel_all().await {
                                Ok(response) => {
                                    warn!(
                                        "BalanceManager: Canceled {} orders on halt",
                                        response.canceled.len()
                                    );
                                }
                                Err(e) => {
                                    warn!("BalanceManager: Failed to cancel orders on halt: {}", e);
                                }
                            }
                        } else if should_resume {
                            // Balance recovered above threshold - resume trading
                            info!(
                                "BalanceManager: RESUMING - balance ${:.2} >= {:.0}% of pivot ${:.2}",
                                new_balance, halt_threshold * 100.0, pivot_val
                            );
                            halt.store(false, Ordering::Release);
                        }

                        debug!(
                            "BalanceManager: current=${:.2}, pivot=${:.2}, threshold=${:.2}, halt={}",
                            new_balance,
                            pivot_val,
                            threshold,
                            halt.load(Ordering::Acquire)
                        );
                    }
                    Err(e) => {
                        warn!("BalanceManager: Failed to fetch balance: {}", e);
                    }
                }
            }
            info!("BalanceManager: Monitoring task stopped");
        });

        self.task_handle = Some(handle);
        Ok(())
    }

    /// Stop the balance monitoring task
    pub async fn stop(&mut self) {
        if let Some(handle) = self.task_handle.take() {
            handle.abort();
            let _ = handle.await;
            info!("BalanceManager stopped");
        }
    }

    /// Check if trading should be halted due to balance drop
    pub fn is_halted(&self) -> bool {
        self.halt_for_trading_balance.load(Ordering::Acquire)
    }

    /// Get current balance
    pub fn current_balance(&self) -> f64 {
        *self.balance_current.read()
    }

    /// Get pivot (high watermark) balance
    pub fn pivot_balance(&self) -> f64 {
        *self.balance_pivot.read()
    }

    /// Get the configured halt threshold (as fraction, e.g., 0.10 for 10%)
    pub fn halt_threshold(&self) -> f64 {
        self.halt_threshold
    }

    /// Get the halt flag Arc for sharing with other components
    pub fn halt_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.halt_for_trading_balance)
    }
}

impl Default for BalanceManager {
    fn default() -> Self {
        Self::new(0.10) // Default to 10% threshold
    }
}
