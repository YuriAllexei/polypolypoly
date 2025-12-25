//! Command types for the Executor.

use crate::application::strategies::inventory_mm::types::{
    SolverOutput, LimitOrder, TakerOrder,
};

/// Commands sent to the Executor thread via channel
#[derive(Debug, Clone)]
pub enum ExecutorCommand {
    /// Execute a full solver output (batch of actions)
    /// This is the primary command - contains cancellations, limit orders, taker orders
    ExecuteBatch(SolverOutput),

    /// Cancel specific orders by ID
    CancelOrders(Vec<String>),

    /// Cancel all orders for a specific token
    CancelAllForToken(String),

    /// Cancel all orders (emergency)
    CancelAll,

    /// Place a single limit order
    PlaceLimit(LimitOrder),

    /// Execute a taker order (FOK)
    ExecuteTaker(TakerOrder),

    /// Graceful shutdown
    Shutdown,
}

impl ExecutorCommand {
    /// Check if this is a shutdown command
    pub fn is_shutdown(&self) -> bool {
        matches!(self, ExecutorCommand::Shutdown)
    }

    /// Get description for logging
    pub fn description(&self) -> &'static str {
        match self {
            ExecutorCommand::ExecuteBatch(_) => "ExecuteBatch",
            ExecutorCommand::CancelOrders(_) => "CancelOrders",
            ExecutorCommand::CancelAllForToken(_) => "CancelAllForToken",
            ExecutorCommand::CancelAll => "CancelAll",
            ExecutorCommand::PlaceLimit(_) => "PlaceLimit",
            ExecutorCommand::ExecuteTaker(_) => "ExecuteTaker",
            ExecutorCommand::Shutdown => "Shutdown",
        }
    }
}

/// Result from executor after processing commands
#[derive(Debug, Clone, Default)]
pub struct ExecutorResult {
    /// Number of orders successfully cancelled
    pub cancelled_count: usize,

    /// Order IDs that were cancelled
    pub cancelled_ids: Vec<String>,

    /// Number of limit orders successfully placed
    pub placed_count: usize,

    /// Order IDs of placed orders
    pub placed_ids: Vec<String>,

    /// Number of taker orders executed
    pub taker_count: usize,

    /// Errors encountered: (context, error message)
    pub errors: Vec<(String, String)>,
}

impl ExecutorResult {
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if all operations succeeded
    pub fn success(&self) -> bool {
        self.errors.is_empty()
    }

    /// Check if any operations failed
    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    /// Total operations performed
    pub fn total_operations(&self) -> usize {
        self.cancelled_count + self.placed_count + self.taker_count
    }

    /// Add an error
    pub fn add_error(&mut self, context: impl Into<String>, message: impl Into<String>) {
        self.errors.push((context.into(), message.into()));
    }

    /// Merge another result into this one
    pub fn merge(&mut self, other: ExecutorResult) {
        self.cancelled_count += other.cancelled_count;
        self.cancelled_ids.extend(other.cancelled_ids);
        self.placed_count += other.placed_count;
        self.placed_ids.extend(other.placed_ids);
        self.taker_count += other.taker_count;
        self.errors.extend(other.errors);
    }
}
