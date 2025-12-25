//! Output types from the solver
//!
//! SolverOutput contains the actions that need to be executed.
//! ExecutorCommand is the message format sent to the Executor thread.

use super::order::{LimitOrder, TakerOrder};

/// Output from the solve() function
/// Contains all actions that need to be executed
#[derive(Debug, Clone, Default)]
pub struct SolverOutput {
    /// Orders to cancel (by order_id)
    pub cancellations: Vec<String>,

    /// Limit orders to place
    pub limit_orders: Vec<LimitOrder>,

    /// Taker orders to execute (time-sensitive, execute first)
    pub taker_orders: Vec<TakerOrder>,
}

impl SolverOutput {
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if there are any actions to execute
    pub fn has_actions(&self) -> bool {
        !self.cancellations.is_empty()
            || !self.limit_orders.is_empty()
            || !self.taker_orders.is_empty()
    }

    /// Total number of actions
    pub fn action_count(&self) -> usize {
        self.cancellations.len() + self.limit_orders.len() + self.taker_orders.len()
    }

    /// Check if output is cancel-only (no new orders)
    pub fn is_cancel_only(&self) -> bool {
        !self.cancellations.is_empty()
            && self.limit_orders.is_empty()
            && self.taker_orders.is_empty()
    }
}

/// Commands sent to the Executor thread via channel
#[derive(Debug, Clone)]
pub enum ExecutorCommand {
    /// Execute a solver output (batch of actions)
    Execute(SolverOutput),

    /// Cancel specific orders
    CancelOrders(Vec<String>),

    /// Cancel all orders for a token
    CancelAllForToken(String),

    /// Place a single limit order
    PlaceLimit(LimitOrder),

    /// Execute a taker order (FOK)
    ExecuteTaker(TakerOrder),

    /// Shutdown the executor
    Shutdown,
}

/// Result from executor after processing a command
#[derive(Debug, Clone)]
pub struct ExecutorResult {
    /// Number of orders successfully cancelled
    pub cancelled_count: usize,

    /// Number of limit orders successfully placed
    pub placed_count: usize,

    /// Number of taker orders executed
    pub taker_count: usize,

    /// Errors encountered (order_id -> error message)
    pub errors: Vec<(String, String)>,
}

impl ExecutorResult {
    pub fn new() -> Self {
        Self {
            cancelled_count: 0,
            placed_count: 0,
            taker_count: 0,
            errors: Vec::new(),
        }
    }

    pub fn success(&self) -> bool {
        self.errors.is_empty()
    }
}

impl Default for ExecutorResult {
    fn default() -> Self {
        Self::new()
    }
}
