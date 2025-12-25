//! Inventory MM Strategy - main orchestration.

use tracing::{info, warn};

use super::config::InventoryMMConfig;
use super::components::{solve, Executor, ExecutorHandle, Merger};
use super::types::{
    SolverInput, SolverOutput, InventorySnapshot, OrderbookSnapshot, OrderSnapshot,
};

/// Main strategy orchestrator
pub struct InventoryMMStrategy {
    config: InventoryMMConfig,
    executor: Option<ExecutorHandle>,
    merger: Option<Merger>,
}

impl InventoryMMStrategy {
    /// Create a new strategy instance
    pub fn new(config: InventoryMMConfig) -> Self {
        Self {
            config,
            executor: None,
            merger: None,
        }
    }

    /// Initialize the strategy (spawn executor, create merger)
    pub fn initialize(&mut self) {
        info!("[InventoryMM] Initializing strategy");

        // Spawn executor on its own thread
        let executor = Executor::spawn();
        self.executor = Some(executor);

        // Create merger
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
    pub fn tick(&self, input: &SolverInput) -> Option<SolverOutput> {
        // Run the pure solver function
        let output = solve(input);

        // Send to executor if we have actions
        if output.has_actions() {
            if let Some(ref executor) = self.executor {
                if let Err(e) = executor.execute(output.clone()) {
                    warn!("[InventoryMM] Failed to send to executor: {}", e);
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
pub fn extract_solver_input(
    config: &InventoryMMConfig,
) -> SolverInput {
    // TODO: Implement extraction from SharedPositionTracker, SharedOrderState, SharedOrderbooks
    SolverInput {
        up_token_id: config.up_token_id.clone(),
        down_token_id: config.down_token_id.clone(),
        up_orders: OrderSnapshot::default(),
        down_orders: OrderSnapshot::default(),
        inventory: InventorySnapshot::default(),
        up_orderbook: OrderbookSnapshot::default(),
        down_orderbook: OrderbookSnapshot::default(),
        config: config.solver.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strategy_lifecycle() {
        let config = InventoryMMConfig::new(
            "up_token".to_string(),
            "down_token".to_string(),
            "condition_123".to_string(),
        );

        let mut strategy = InventoryMMStrategy::new(config.clone());
        strategy.initialize();

        // Run a tick with placeholder input
        let input = extract_solver_input(&config);
        let output = strategy.tick(&input);

        assert!(output.is_some());

        // Shutdown
        strategy.shutdown();
    }
}
