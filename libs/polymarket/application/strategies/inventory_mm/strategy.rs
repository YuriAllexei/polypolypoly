//! Inventory MM Strategy - main orchestration.

use std::sync::Arc;
use tracing::{info, warn};

use super::config::InventoryMMConfig;
use super::components::{solve, Executor, ExecutorHandle, Merger, InFlightTracker, OpenOrderInfo};
use super::types::{
    SolverInput, SolverOutput, InventorySnapshot, OrderbookSnapshot, OrderSnapshot, OpenOrder,
};
use crate::infrastructure::{
    SharedOrderbooks, SharedOrderState, SharedPositionTracker, UserOrderStatus as OrderStatus,
};
use crate::infrastructure::client::clob::TradingClient;

/// Main strategy orchestrator
pub struct InventoryMMStrategy {
    config: InventoryMMConfig,
    executor: Option<ExecutorHandle>,
    merger: Option<Merger>,
    in_flight_tracker: InFlightTracker,
}

impl InventoryMMStrategy {
    /// Create a new strategy instance
    pub fn new(config: InventoryMMConfig) -> Self {
        Self {
            config,
            executor: None,
            merger: None,
            in_flight_tracker: InFlightTracker::with_default_ttl(),
        }
    }

    /// Initialize the strategy with a trading client
    pub fn initialize(&mut self, trading: Arc<TradingClient>) {
        info!("[InventoryMM] Initializing strategy");

        let executor = Executor::spawn(trading);
        self.executor = Some(executor);

        let merger = Merger::new(
            self.config.merger.clone(),
            self.config.up_token_id.clone(),
            self.config.down_token_id.clone(),
            self.config.condition_id.clone(),
        );
        self.merger = Some(merger);

        info!("[InventoryMM] Strategy initialized");
    }

    /// Run one iteration of the strategy
    ///
    /// This should be called periodically (e.g., every 100ms)
    pub fn tick(&mut self, input: &SolverInput) -> Option<SolverOutput> {
        // 1. Cleanup stale in-flight entries based on current OMS state
        let open_orders: Vec<OpenOrderInfo> = input.up_orders.bids.iter()
            .map(|o| OpenOrderInfo::new(o.order_id.clone(), input.up_token_id.clone(), o.price))
            .chain(input.down_orders.bids.iter()
                .map(|o| OpenOrderInfo::new(o.order_id.clone(), input.down_token_id.clone(), o.price)))
            .collect();
        self.in_flight_tracker.cleanup_from_orders(&open_orders);

        // 2. Run the pure solver function
        let mut output = solve(input);

        // 3. Filter cancellations through in-flight tracker
        output.cancellations.retain(|oid| self.in_flight_tracker.should_cancel(oid));

        // 4. Filter placements through in-flight tracker
        output.limit_orders.retain(|o| self.in_flight_tracker.should_place(&o.token_id, o.price));

        // 5. Taker orders pass through (no filtering - they're immediate)

        // 6. Send filtered output to executor
        if output.has_actions() {
            if let Some(ref executor) = self.executor {
                if let Err(e) = executor.execute(output.clone()) {
                    warn!("[InventoryMM] Failed to send to executor: {}", e);

                    // Unregister failed commands so they can retry immediately
                    for oid in &output.cancellations {
                        self.in_flight_tracker.cancel_failed(oid);
                    }
                    for order in &output.limit_orders {
                        self.in_flight_tracker.placement_failed(&order.token_id, order.price);
                    }
                }
            }
        }

        // Check merger
        if let Some(ref merger) = self.merger {
            let decision = merger.check_merge(&input.inventory);
            if decision.should_merge {
                info!(
                    "[InventoryMM] Merge opportunity: {} pairs for ${:.4} profit",
                    decision.pairs_to_merge, decision.expected_profit
                );
                // TODO: Execute merge via API
            }
        }

        Some(output)
    }

    /// Shutdown the strategy
    pub fn shutdown(mut self) {
        info!("[InventoryMM] Shutting down strategy");

        if let Some(executor) = self.executor.take() {
            if let Err(e) = executor.shutdown() {
                warn!("[InventoryMM] Error shutting down executor: {}", e);
            }
        }

        info!("[InventoryMM] Strategy shutdown complete");
    }

    /// Get config reference
    pub fn config(&self) -> &InventoryMMConfig {
        &self.config
    }
}

/// Extract SolverInput from shared state.
///
/// Acquires read locks in order: OMS → PositionTracker → Orderbooks (prevent deadlocks).
pub fn extract_solver_input(
    config: &InventoryMMConfig,
    orderbooks: &SharedOrderbooks,
    order_state: &SharedOrderState,
    position_tracker: &SharedPositionTracker,
) -> SolverInput {
    // 1. Extract open orders from OMS
    let (up_orders, down_orders) = {
        let oms = order_state.read();

        let extract_orders = |token_id: &str| -> OrderSnapshot {
            let bids: Vec<OpenOrder> = oms.get_bids(token_id)
                .iter()
                .filter(|o| o.status == OrderStatus::Open || o.status == OrderStatus::PartiallyFilled)
                .map(|o| OpenOrder::new(
                    o.order_id.clone(),
                    o.price,
                    o.original_size,
                    o.original_size - o.size_matched,
                ))
                .collect();

            OrderSnapshot { bids, asks: vec![] }
        };

        (extract_orders(&config.up_token_id), extract_orders(&config.down_token_id))
    };

    // 2. Extract inventory from position tracker
    let inventory = {
        let tracker = position_tracker.read();
        let up_pos = tracker.get_position(&config.up_token_id);
        let down_pos = tracker.get_position(&config.down_token_id);

        InventorySnapshot {
            up_size: up_pos.map(|p| p.size).unwrap_or(0.0),
            up_avg_price: up_pos.map(|p| p.avg_entry_price).unwrap_or(0.0),
            down_size: down_pos.map(|p| p.size).unwrap_or(0.0),
            down_avg_price: down_pos.map(|p| p.avg_entry_price).unwrap_or(0.0),
        }
    };

    // 3. Extract orderbook snapshots
    let (up_orderbook, down_orderbook) = {
        let obs = orderbooks.read();

        let extract_ob = |token_id: &str, our_orders: &OrderSnapshot| -> OrderbookSnapshot {
            match obs.get(token_id) {
                Some(ob) => {
                    let best_bid = ob.best_bid();
                    let best_ask = ob.best_ask();

                    let best_bid_is_ours = best_bid
                        .map(|(price, _)| our_orders.bids.iter().any(|o| (o.price - price).abs() < 1e-6))
                        .unwrap_or(false);

                    let best_ask_is_ours = best_ask
                        .map(|(price, _)| our_orders.asks.iter().any(|o| (o.price - price).abs() < 1e-6))
                        .unwrap_or(false);

                    OrderbookSnapshot { best_bid, best_ask, best_bid_is_ours, best_ask_is_ours }
                }
                None => OrderbookSnapshot::default(),
            }
        };

        (extract_ob(&config.up_token_id, &up_orders), extract_ob(&config.down_token_id, &down_orders))
    };

    SolverInput {
        up_token_id: config.up_token_id.clone(),
        down_token_id: config.down_token_id.clone(),
        up_orders,
        down_orders,
        inventory,
        up_orderbook,
        down_orderbook,
        config: config.solver.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use parking_lot::RwLock;
    use crate::infrastructure::{OrderStateStore, PositionTracker};

    #[test]
    fn test_extract_solver_input() {
        let config = InventoryMMConfig::new(
            "up_token".to_string(),
            "down_token".to_string(),
            "condition_123".to_string(),
        );

        let orderbooks = Arc::new(RwLock::new(HashMap::new()));
        let order_state = Arc::new(RwLock::new(OrderStateStore::new()));
        let position_tracker = Arc::new(RwLock::new(PositionTracker::new()));

        let input = extract_solver_input(&config, &orderbooks, &order_state, &position_tracker);

        assert_eq!(input.up_token_id, "up_token");
        assert_eq!(input.down_token_id, "down_token");
        assert!(input.up_orders.bids.is_empty());
        assert!(input.down_orders.bids.is_empty());
    }

    // Full lifecycle test requires TradingClient - run as integration test
    // #[test]
    // #[ignore]
    // fn test_strategy_lifecycle() { ... }
}
