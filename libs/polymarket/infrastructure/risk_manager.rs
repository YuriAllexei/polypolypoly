//! Standalone Risk Manager Component
//!
//! Provides:
//! - Background monitoring thread that cancels orders when oracle price
//!   approaches price_to_beat
//! - Fast pre_placement_check() for synchronous risk validation
//!
//! The RiskManager runs independently on its own OS thread and can be used
//! by any strategy that needs oracle-based risk management.

use crate::application::strategies::up_or_down::types::{CryptoAsset, OracleSource};
use crate::infrastructure::client::clob::TradingClient;
use crate::infrastructure::SharedOraclePrices;
use chrono::{DateTime, Utc};
use crossbeam_channel::{Receiver, Sender, TryRecvError};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tracing::{debug, error, info, warn};

// =============================================================================
// Types
// =============================================================================

/// Commands sent to the RiskManager background thread
#[derive(Debug, Clone)]
pub enum RiskManagerCommand {
    /// Register a market for continuous monitoring
    RegisterMarket {
        market_id: String,
        price_to_beat: f64,
        oracle_source: OracleSource,
        crypto_asset: CryptoAsset,
        market_end_time: DateTime<Utc>,
        token_ids: [String; 2],
    },
    /// Shutdown the risk manager
    Shutdown,
}

/// Internal representation of a registered market
#[derive(Debug, Clone)]
struct RegisteredMarket {
    market_id: String,
    price_to_beat: f64,
    oracle_source: OracleSource,
    crypto_asset: CryptoAsset,
    market_end_time: DateTime<Utc>,
    token_ids: [String; 2],
}

/// Shared registry of markets being monitored
type SharedMarketRegistry = Arc<RwLock<HashMap<String, RegisteredMarket>>>;

// =============================================================================
// RiskManagerHandle
// =============================================================================

/// Handle for communicating with the RiskManager
///
/// This handle is Clone and can be shared across threads.
/// It provides methods to register markets and perform pre-placement checks.
#[derive(Clone)]
pub struct RiskManagerHandle {
    tx: Sender<RiskManagerCommand>,
    registry: SharedMarketRegistry,
    oracle_prices: SharedOraclePrices,
    bps_threshold: f64,
}

impl RiskManagerHandle {
    /// Register a market for continuous monitoring
    ///
    /// The background thread will monitor this market and cancel orders
    /// if the oracle price gets too close to the price_to_beat.
    pub fn register_market(
        &self,
        market_id: String,
        price_to_beat: f64,
        oracle_source: OracleSource,
        crypto_asset: CryptoAsset,
        market_end_time: DateTime<Utc>,
        token_ids: [String; 2],
    ) -> Result<(), crossbeam_channel::SendError<RiskManagerCommand>> {
        self.tx.send(RiskManagerCommand::RegisterMarket {
            market_id,
            price_to_beat,
            oracle_source,
            crypto_asset,
            market_end_time,
            token_ids,
        })
    }

    /// Synchronous pre-placement risk check
    ///
    /// Returns true if SAFE to place order, false if RISKY.
    /// Checks: |price_to_beat - oracle_price| / price_to_beat * 10000 >= bps_threshold
    ///
    /// This is a fast path that only reads from shared oracle prices.
    pub fn pre_placement_check(
        &self,
        price_to_beat: f64,
        oracle_source: OracleSource,
        crypto_asset: CryptoAsset,
    ) -> bool {
        // Get oracle type
        let oracle_type = match oracle_source.to_oracle_type() {
            Some(ot) => ot,
            None => {
                error!("pre_placement_check: Unknown oracle source, rejecting order");
                return false;
            }
        };

        // Get crypto symbol
        let symbol = match crypto_asset.oracle_symbol() {
            Some(s) => s,
            None => {
                error!("pre_placement_check: Unknown crypto asset, rejecting order");
                return false;
            }
        };

        // Get oracle price (read lock only)
        let oracle_price = {
            let manager = self.oracle_prices.read();
            manager.get_price(oracle_type, symbol).map(|e| e.value)
        };

        let Some(oracle_price) = oracle_price else {
            // No oracle price available - allow disallow order (fail open)
            error!("pre_placement_check: No oracle price available, rejecting order");
            return false;
        };

        // Calculate BPS difference
        let bps_diff = ((price_to_beat - oracle_price).abs() / price_to_beat) * 10000.0;

        // Safe if BPS diff >= threshold
        let is_safe = bps_diff >= self.bps_threshold;

        if !is_safe {
            info!(
                "pre_placement_check FAIL: Oracle ${:.2} vs target ${:.2} ({:.1} bps < {:.1} threshold)",
                oracle_price, price_to_beat, bps_diff, self.bps_threshold
            );
        }

        is_safe
    }

    /// Send shutdown signal to the background thread
    pub fn shutdown(&self) {
        let _ = self.tx.send(RiskManagerCommand::Shutdown);
    }

    /// Get the number of currently registered markets
    pub fn market_count(&self) -> usize {
        self.registry.read().len()
    }
}

// =============================================================================
// RiskManager
// =============================================================================

/// Risk Manager that runs on a dedicated OS thread
pub struct RiskManager;

impl RiskManager {
    /// Spawn the risk manager on a dedicated OS thread
    ///
    /// Returns a handle for communication and control.
    ///
    /// # Arguments
    /// - `trading_client`: Client for cancelling orders (Arc-wrapped)
    /// - `oracle_prices`: Shared oracle price state
    /// - `bps_threshold`: BPS threshold for risk detection
    pub fn spawn(
        trading_client: Arc<TradingClient>,
        oracle_prices: SharedOraclePrices,
        bps_threshold: f64,
    ) -> RiskManagerHandle {
        let (tx, rx) = crossbeam_channel::unbounded();
        let registry: SharedMarketRegistry = Arc::new(RwLock::new(HashMap::new()));

        let handle = RiskManagerHandle {
            tx,
            registry: registry.clone(),
            oracle_prices: oracle_prices.clone(),
            bps_threshold,
        };

        // Spawn background monitoring thread
        thread::Builder::new()
            .name("risk-manager".to_string())
            .spawn(move || {
                info!("RiskManager background thread started");
                run_loop(rx, registry, oracle_prices, trading_client, bps_threshold);
                info!("RiskManager background thread stopped");
            })
            .expect("Failed to spawn risk manager thread");

        handle
    }
}

// =============================================================================
// Background Thread Loop
// =============================================================================

fn run_loop(
    rx: Receiver<RiskManagerCommand>,
    registry: SharedMarketRegistry,
    oracle_prices: SharedOraclePrices,
    trading_client: Arc<TradingClient>,
    bps_threshold: f64,
) {
    // Create a single-threaded tokio runtime for async calls
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime for risk manager");

    loop {
        // 1. Drain ALL pending commands (non-blocking)
        loop {
            match rx.try_recv() {
                Ok(RiskManagerCommand::RegisterMarket {
                    market_id,
                    price_to_beat,
                    oracle_source,
                    crypto_asset,
                    market_end_time,
                    token_ids,
                }) => {
                    info!(
                        "Registering market {} for risk monitoring (price_to_beat: ${:.2}, threshold: {:.1} bps)",
                        market_id, price_to_beat, bps_threshold
                    );
                    let mut reg = registry.write();
                    reg.insert(
                        market_id.clone(),
                        RegisteredMarket {
                            market_id,
                            price_to_beat,
                            oracle_source,
                            crypto_asset,
                            market_end_time,
                            token_ids,
                        },
                    );
                }
                Ok(RiskManagerCommand::Shutdown) => {
                    info!("RiskManager received shutdown signal");
                    return;
                }
                Err(TryRecvError::Empty) => break, // No more commands
                Err(TryRecvError::Disconnected) => {
                    info!("RiskManager channel disconnected, shutting down");
                    return;
                }
            }
        }

        // 2. Get snapshot of registered markets
        let markets_snapshot: Vec<RegisteredMarket> =
            { registry.read().values().cloned().collect() };

        // Skip if no markets to check
        if markets_snapshot.is_empty() {
            // Sleep to prevent busy-spin when idle
            thread::sleep(Duration::from_millis(1));
            continue;
        }

        let mut to_remove = Vec::new();
        let now = Utc::now();

        // 3. Check all registered markets
        for market in markets_snapshot {
            // Check if market ended
            if now > market.market_end_time {
                info!("Unregistering market {} - market ended", market.market_id);
                to_remove.push(market.market_id.clone());
                continue;
            }

            // Get oracle type and symbol
            let oracle_type = match market.oracle_source.to_oracle_type() {
                Some(ot) => ot,
                None => continue,
            };
            let symbol = match market.crypto_asset.oracle_symbol() {
                Some(s) => s,
                None => continue,
            };

            // Get oracle price
            let oracle_price = {
                let manager = oracle_prices.read();
                manager.get_price(oracle_type, symbol).map(|e| e.value)
            };

            let Some(oracle_price) = oracle_price else {
                continue;
            };

            // Calculate BPS difference
            let bps_diff =
                ((market.price_to_beat - oracle_price).abs() / market.price_to_beat) * 10000.0;

            // If below threshold, cancel orders for BOTH token IDs
            if bps_diff < bps_threshold {
                info!(
                    "Market {} - BPS diff {:.2} below threshold {:.2}, cancelling orders",
                    market.market_id, bps_diff, bps_threshold
                );

                for token_id in &market.token_ids {
                    cancel_orders_for_token(&runtime, &trading_client, token_id, &market.market_id);
                }
                // NOTE: Keep monitoring - don't remove from registry
            }
        }

        // 4. Remove ended markets
        if !to_remove.is_empty() {
            let mut reg = registry.write();
            for market_id in to_remove {
                reg.remove(&market_id);
            }
        }

        // 5. Sleep to prevent busy-spin burning 100% CPU
        thread::sleep(Duration::from_millis(1));
    }
}

/// Cancel orders for a specific token ID
fn cancel_orders_for_token(
    runtime: &tokio::runtime::Runtime,
    trading_client: &Arc<TradingClient>,
    token_id: &str,
    market_id: &str,
) {
    info!(
        "Cancelling orders for token {} in market {}",
        token_id, market_id
    );

    // Use block_on to call the async cancel method
    let result = runtime.block_on(async {
        trading_client
            .cancel_market_orders(None, Some(token_id))
            .await
    });

    match result {
        Ok(response) => {
            if !response.canceled.is_empty() {
                info!(
                    "Successfully cancelled {} orders for token {}",
                    response.canceled.len(),
                    token_id
                );
            }
            if !response.not_canceled.is_empty() {
                warn!(
                    "Failed to cancel {} orders for token {}: {:?}",
                    response.not_canceled.len(),
                    token_id,
                    response.not_canceled
                );
            }
        }
        Err(e) => {
            error!("Failed to cancel orders for token {}: {}", token_id, e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Basic compile-time test to ensure types are correct
    #[test]
    fn test_types_compile() {
        // Just ensure the types are correct
        let _: fn(CryptoAsset) -> Option<&'static str> = |ca| ca.oracle_symbol();
    }
}
