//! Output types from the solver
//!
//! SolverOutput contains the actions that need to be executed.
//! ExecutorCommand is the message format sent to the Executor thread.

use super::order::LimitOrder;

/// Output from the solve() function
/// Contains all actions that need to be executed
#[derive(Debug, Clone, Default)]
pub struct SolverOutput {
    /// Orders to cancel (by order_id)
    pub cancellations: Vec<String>,

    /// Limit orders to place
    pub limit_orders: Vec<LimitOrder>,
}

impl SolverOutput {
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if there are any actions to execute
    pub fn has_actions(&self) -> bool {
        !self.cancellations.is_empty() || !self.limit_orders.is_empty()
    }

    /// Total number of actions
    pub fn action_count(&self) -> usize {
        self.cancellations.len() + self.limit_orders.len()
    }

    /// Check if output is cancel-only (no new orders)
    pub fn is_cancel_only(&self) -> bool {
        !self.cancellations.is_empty() && self.limit_orders.is_empty()
    }
}

