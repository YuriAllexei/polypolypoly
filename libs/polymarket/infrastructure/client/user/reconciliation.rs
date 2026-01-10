//! Reconciliation Tasks
//!
//! Provides periodic REST API reconciliation for positions and orders.
//! REST API is treated as authoritative source of truth, correcting
//! any drift from WebSocket-based real-time tracking.
//!
//! ## Usage
//!
//! ```ignore
//! use polymarket::infrastructure::client::user::*;
//!
//! // Position reconciliation
//! let pos_handle = spawn_position_reconciliation_task(
//!     shutdown_flag.clone(),
//!     position_tracker,
//!     trading_client.clone(),
//!     ReconciliationConfig::with_interval(1),
//! );
//!
//! // Order reconciliation
//! let order_handle = spawn_order_reconciliation_task(
//!     shutdown_flag,
//!     order_state,
//!     trading_client,
//!     ReconciliationConfig::with_interval(1),
//! );
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout, Duration};
use tracing::{debug, error, info, warn};

use crate::infrastructure::client::clob::TradingClient;

use super::{SharedOrderState, SharedPositionTracker};

// =============================================================================
// Configuration
// =============================================================================

/// Configuration for the position reconciliation task
#[derive(Debug, Clone)]
pub struct ReconciliationConfig {
    /// Whether reconciliation is enabled
    pub enabled: bool,
    /// Interval between reconciliation attempts in seconds
    pub interval_secs: u64,
}

impl Default for ReconciliationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_secs: 3,
        }
    }
}

impl ReconciliationConfig {
    /// Create a new config with custom interval
    pub fn with_interval(interval_secs: u64) -> Self {
        Self {
            enabled: true,
            interval_secs,
        }
    }

    /// Create a disabled config
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            interval_secs: 3,
        }
    }
}

// =============================================================================
// Reconciliation Task
// =============================================================================

/// Spawns a background task that periodically reconciles positions with REST API
///
/// The task fetches positions from the REST API at the configured interval
/// and corrects any discrepancies in the local position tracker.
///
/// # Arguments
/// * `shutdown_flag` - Atomic flag to signal shutdown
/// * `tracker` - Shared position tracker to reconcile
/// * `trading` - Trading client for REST API calls
/// * `config` - Reconciliation configuration
///
/// # Returns
/// * `Some(JoinHandle)` if enabled, `None` if disabled
pub fn spawn_position_reconciliation_task(
    shutdown_flag: Arc<AtomicBool>,
    tracker: SharedPositionTracker,
    trading: Arc<TradingClient>,
    config: ReconciliationConfig,
) -> Option<JoinHandle<()>> {
    if !config.enabled {
        info!("[Reconciliation] Task disabled");
        return None;
    }

    Some(tokio::spawn(async move {
        let interval_duration = Duration::from_secs(config.interval_secs);

        info!(
            "[Reconciliation] Task started (interval: {}s)",
            config.interval_secs
        );

        // Initial delay before first reconciliation to let WebSocket stabilize
        sleep(interval_duration).await;

        while shutdown_flag.load(Ordering::Acquire) {
            // Fetch positions from REST API using trading client
            // Use maker_address (proxy wallet) - that's where positions are held
            match trading.rest().get_positions(trading.maker_address()).await {
                Ok(positions) => {
                    // Convert REST positions to (token_id, size, avg_price) tuples
                    // CLOB Position type: asset_id: String, size: String, avg_price: Option<f64>
                    let rest_positions: Vec<(String, f64, f64)> = positions
                        .iter()
                        .filter_map(|p| {
                            let size = p.size.parse::<f64>().ok()?;
                            if size.abs() < 0.001 {
                                return None; // Skip dust positions
                            }
                            let price = p.avg_price.unwrap_or(0.0);
                            Some((p.asset_id.clone(), size, price))
                        })
                        .collect();

                    // Reconcile (acquire write lock)
                    let result = tracker.write().reconcile(&rest_positions);

                    if result.has_discrepancies() {
                        warn!(
                            "[Reconciliation] Corrected {} discrepancies:",
                            result.discrepancies.len()
                        );
                        for d in &result.discrepancies {
                            let token_short = if d.token_id.len() > 8 {
                                &d.token_id[..8]
                            } else {
                                &d.token_id
                            };
                            warn!(
                                "  {}...: tracked={:.2} -> REST={:.2} (diff={:+.2})",
                                token_short,
                                d.tracked_size,
                                d.rest_size,
                                d.size_diff()
                            );
                        }
                    } else {
                        debug!(
                            "[Reconciliation] OK - {} positions verified",
                            result.positions_checked
                        );
                    }
                }
                Err(e) => {
                    warn!("[Reconciliation] REST fetch failed: {}", e);
                }
            }

            // Wait before next reconciliation
            // Using sleep() instead of interval() prevents overlapping reconciliations
            // when REST API is slow
            sleep(interval_duration).await;
        }

        info!("[Reconciliation] Task shutting down");
    }))
}

// =============================================================================
// Order Reconciliation Task
// =============================================================================

/// REST API request timeout
const ORDER_REST_TIMEOUT_SECS: u64 = 30;

/// Maximum consecutive failures before logging as error
const ORDER_MAX_CONSECUTIVE_FAILURES: u32 = 5;

/// Maximum backoff duration in seconds
const ORDER_MAX_BACKOFF_SECS: u64 = 60;

/// Spawns a background task that periodically reconciles orders with REST API
///
/// The task fetches open orders from the REST API at the configured interval
/// and removes any stale orders that no longer exist on the exchange.
///
/// Features:
/// - Exponential backoff on consecutive REST API failures
/// - Request timeout to prevent hanging
/// - Escalates to error log after MAX_CONSECUTIVE_FAILURES
///
/// # Arguments
/// * `shutdown_flag` - Atomic flag to signal shutdown
/// * `order_state` - Shared order state to reconcile
/// * `trading` - Trading client for REST API calls
/// * `config` - Reconciliation configuration
///
/// # Returns
/// * `Some(JoinHandle)` if enabled, `None` if disabled
pub fn spawn_order_reconciliation_task(
    shutdown_flag: Arc<AtomicBool>,
    order_state: SharedOrderState,
    trading: Arc<TradingClient>,
    config: ReconciliationConfig,
) -> Option<JoinHandle<()>> {
    if !config.enabled {
        info!("[OrderReconciliation] Task disabled");
        return None;
    }

    Some(tokio::spawn(async move {
        let base_interval = Duration::from_secs(config.interval_secs);
        let rest_timeout = Duration::from_secs(ORDER_REST_TIMEOUT_SECS);

        info!(
            "[OrderReconciliation] Task started (interval: {}s, timeout: {}s)",
            config.interval_secs, ORDER_REST_TIMEOUT_SECS
        );

        // Initial delay before first reconciliation to let WebSocket stabilize
        // Use 2x base interval for better stability
        sleep(base_interval * 2).await;

        let mut consecutive_failures: u32 = 0;

        while shutdown_flag.load(Ordering::Acquire) {
            // Fetch open orders from REST API with timeout
            let fetch_result = timeout(rest_timeout, trading.get_orders(None)).await;

            match fetch_result {
                Ok(Ok(orders)) => {
                    // Success - reset failure counter
                    if consecutive_failures > 0 {
                        info!(
                            "[OrderReconciliation] REST API recovered after {} failures",
                            consecutive_failures
                        );
                    }
                    consecutive_failures = 0;

                    // Reconcile (acquire write lock)
                    let result = order_state.write().reconcile_orders(&orders);

                    if result.has_discrepancies() {
                        warn!(
                            "[OrderReconciliation] Removed {} stale orders:",
                            result.stale_orders_removed
                        );
                        for order_id in &result.removed_order_ids {
                            let short_id = if order_id.len() > 16 {
                                &order_id[..16]
                            } else {
                                order_id
                            };
                            warn!("  {}... (not in REST)", short_id);
                        }
                    } else {
                        debug!(
                            "[OrderReconciliation] OK - {} orders verified",
                            result.orders_checked
                        );
                    }
                }
                Ok(Err(e)) => {
                    // REST API returned an error
                    consecutive_failures += 1;
                    if consecutive_failures >= ORDER_MAX_CONSECUTIVE_FAILURES {
                        error!(
                            "[OrderReconciliation] REST fetch failed ({} consecutive): {}",
                            consecutive_failures, e
                        );
                    } else {
                        warn!(
                            "[OrderReconciliation] REST fetch failed ({}/{}): {}",
                            consecutive_failures, ORDER_MAX_CONSECUTIVE_FAILURES, e
                        );
                    }
                }
                Err(_) => {
                    // Timeout
                    consecutive_failures += 1;
                    if consecutive_failures >= ORDER_MAX_CONSECUTIVE_FAILURES {
                        error!(
                            "[OrderReconciliation] REST fetch timed out after {}s ({} consecutive)",
                            ORDER_REST_TIMEOUT_SECS, consecutive_failures
                        );
                    } else {
                        warn!(
                            "[OrderReconciliation] REST fetch timed out after {}s ({}/{})",
                            ORDER_REST_TIMEOUT_SECS, consecutive_failures, ORDER_MAX_CONSECUTIVE_FAILURES
                        );
                    }
                }
            }

            // Calculate wait duration with exponential backoff on failures
            let wait_duration = if consecutive_failures > 0 {
                // Exponential backoff: base * 2^(failures-1), capped at MAX_BACKOFF
                let backoff_secs = config.interval_secs
                    .saturating_mul(1 << consecutive_failures.min(6))
                    .min(ORDER_MAX_BACKOFF_SECS);
                Duration::from_secs(backoff_secs)
            } else {
                base_interval
            };

            sleep(wait_duration).await;
        }

        info!("[OrderReconciliation] Task shutting down");
    }))
}
