//! Executor - runs on its own thread, processes commands via channel.

use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use crossbeam_channel::{unbounded, Sender, Receiver};
use tokio::runtime::Runtime;
use tracing::{info, warn, error, debug};

use super::commands::{ExecutorCommand, ExecutorResult};
use crate::application::strategies::inventory_mm::types::{SolverOutput, LimitOrder, Side};
use crate::infrastructure::client::clob::{TradingClient, Side as TradingSide, OrderType};
use crate::infrastructure::client::ctf::{merge as ctf_merge, usdc_to_raw};
use crate::infrastructure::SharedOrderState;

/// Lightweight executor handle for quoters (Clone-able).
/// Does NOT have shutdown capability - only main strategy can shutdown.
#[derive(Clone)]
pub struct QuoterExecutorHandle {
    command_tx: Sender<ExecutorCommand>,
}

impl QuoterExecutorHandle {
    /// Create from a raw sender (for testing).
    #[cfg(test)]
    pub fn from_sender(command_tx: Sender<ExecutorCommand>) -> Self {
        Self { command_tx }
    }

    /// Execute a solver output (non-blocking send to executor).
    pub fn execute(&self, output: SolverOutput) -> Result<(), ExecutorError> {
        if !output.has_actions() {
            return Ok(());
        }
        self.command_tx
            .send(ExecutorCommand::ExecuteBatch(output))
            .map_err(|_| ExecutorError::ChannelClosed)
    }

    /// Cancel specific orders.
    pub fn cancel_orders(&self, order_ids: Vec<String>) -> Result<(), ExecutorError> {
        if order_ids.is_empty() {
            return Ok(());
        }
        self.command_tx
            .send(ExecutorCommand::CancelOrders(order_ids))
            .map_err(|_| ExecutorError::ChannelClosed)
    }

    /// Cancel all orders for a specific token.
    pub fn cancel_token_orders(&self, token_id: String) -> Result<(), ExecutorError> {
        self.command_tx
            .send(ExecutorCommand::CancelAllForToken(token_id))
            .map_err(|_| ExecutorError::ChannelClosed)
    }

    /// Execute a merge (convert YES+NO tokens to USDC).
    pub fn merge(&self, condition_id: String, amount: f64) -> Result<(), ExecutorError> {
        self.command_tx
            .send(ExecutorCommand::Merge { condition_id, amount })
            .map_err(|_| ExecutorError::ChannelClosed)
    }
}

/// Handle to communicate with the Executor thread.
/// Owned by the main strategy - has shutdown capability.
pub struct ExecutorHandle {
    /// Channel to send commands to executor
    command_tx: Sender<ExecutorCommand>,
    /// Thread handle for joining on shutdown
    thread_handle: Option<JoinHandle<()>>,
}

impl ExecutorHandle {
    /// Get a lightweight clone-able handle for quoters.
    /// Quoters use this to send commands without shutdown capability.
    pub fn quoter_handle(&self) -> QuoterExecutorHandle {
        QuoterExecutorHandle {
            command_tx: self.command_tx.clone(),
        }
    }

    /// Send a command to the executor (non-blocking)
    pub fn send(&self, command: ExecutorCommand) -> Result<(), ExecutorError> {
        self.command_tx
            .send(command)
            .map_err(|_| ExecutorError::ChannelClosed)
    }

    /// Execute a solver output
    pub fn execute(&self, output: SolverOutput) -> Result<(), ExecutorError> {
        if !output.has_actions() {
            return Ok(()); // Nothing to do
        }
        self.send(ExecutorCommand::ExecuteBatch(output))
    }

    /// Cancel specific orders
    pub fn cancel_orders(&self, order_ids: Vec<String>) -> Result<(), ExecutorError> {
        if order_ids.is_empty() {
            return Ok(());
        }
        self.send(ExecutorCommand::CancelOrders(order_ids))
    }

    /// Emergency cancel all
    pub fn cancel_all(&self) -> Result<(), ExecutorError> {
        self.send(ExecutorCommand::CancelAll)
    }

    /// Execute a merge (convert YES+NO tokens to USDC)
    pub fn merge(&self, condition_id: String, amount: f64) -> Result<(), ExecutorError> {
        self.send(ExecutorCommand::Merge { condition_id, amount })
    }

    /// Shutdown the executor gracefully with timeout
    pub fn shutdown(mut self) -> Result<(), ExecutorError> {
        let _ = self.send(ExecutorCommand::Shutdown);

        if let Some(handle) = self.thread_handle.take() {
            let timeout = Duration::from_secs(10);
            let start = Instant::now();

            // Poll for thread completion with timeout
            while !handle.is_finished() {
                if start.elapsed() > timeout {
                    warn!("[Executor] Shutdown timeout after 10s, thread still running");
                    return Err(ExecutorError::ShutdownTimeout);
                }
                std::thread::sleep(Duration::from_millis(50));
            }

            handle.join().map_err(|_| ExecutorError::ThreadPanic)?;
        }

        Ok(())
    }

    /// Check if executor is still running
    pub fn is_running(&self) -> bool {
        self.thread_handle
            .as_ref()
            .map(|h| !h.is_finished())
            .unwrap_or(false)
    }
}

impl Drop for ExecutorHandle {
    fn drop(&mut self) {
        // Send shutdown command if channel is still open
        let _ = self.command_tx.send(ExecutorCommand::Shutdown);

        // Wait for thread to finish
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }
}

/// The Executor that runs on its own thread
pub struct Executor {
    /// Receiver for commands
    command_rx: Receiver<ExecutorCommand>,
    /// Trading client for order execution
    trading: Arc<TradingClient>,
    /// Tokio runtime for async calls
    runtime: Runtime,
    /// Shared order state for optimistic updates after REST confirms cancellations.
    /// This fixes the issue where WebSocket CANCELLATION messages are delayed/dropped
    /// causing the OMS to keep stale "Open" order status.
    order_state: Option<SharedOrderState>,
}

impl Executor {
    /// Spawn the executor on a new thread with a trading client.
    /// Optionally accepts SharedOrderState for optimistic OMS updates when cancels are confirmed.
    pub fn spawn(trading: Arc<TradingClient>) -> ExecutorHandle {
        Self::spawn_with_order_state(trading, None)
    }

    /// Spawn the executor with SharedOrderState for optimistic OMS updates.
    /// When the REST API confirms cancellations, the executor will update the OMS directly
    /// instead of waiting for WebSocket CANCELLATION messages (which may be delayed/dropped).
    pub fn spawn_with_order_state(trading: Arc<TradingClient>, order_state: Option<SharedOrderState>) -> ExecutorHandle {
        let (command_tx, command_rx) = unbounded();

        let runtime = Runtime::new().expect("Failed to create tokio runtime");

        let executor = Executor {
            command_rx,
            trading,
            runtime,
            order_state,
        };

        let thread_handle = thread::Builder::new()
            .name("inventory-mm-executor".to_string())
            .spawn(move || {
                executor.run();
            })
            .expect("Failed to spawn executor thread");

        ExecutorHandle {
            command_tx,
            thread_handle: Some(thread_handle),
        }
    }

    /// Main run loop - blocks on channel, processes commands
    fn run(self) {
        info!("[Executor] Started on thread {:?}", thread::current().id());

        loop {
            // Block waiting for command (efficient - no busy polling)
            match self.command_rx.recv() {
                Ok(command) => {
                    debug!("[Executor] Received command: {}", command.description());

                    if command.is_shutdown() {
                        info!("[Executor] Shutdown command received, exiting");
                        break;
                    }

                    let result = self.process_command(command);

                    if result.has_errors() {
                        for (context, err) in &result.errors {
                            error!("[Executor] Error in {}: {}", context, err);
                        }
                    } else {
                        debug!(
                            "[Executor] Completed: cancelled={}, placed={}",
                            result.cancelled_count, result.placed_count
                        );
                    }
                }
                Err(_) => {
                    // Channel closed, shutdown
                    info!("[Executor] Channel closed, shutting down");
                    break;
                }
            }
        }

        info!("[Executor] Thread exiting");
    }

    /// Process a single command
    fn process_command(&self, command: ExecutorCommand) -> ExecutorResult {
        let mut result = ExecutorResult::new();

        match command {
            ExecutorCommand::ExecuteBatch(output) => {
                // 1. Cancellations first
                if !output.cancellations.is_empty() {
                    result.merge(self.execute_cancellations(&output.cancellations));
                }

                // 2. Limits (batch)
                if !output.limit_orders.is_empty() {
                    result.merge(self.execute_limits(&output.limit_orders));
                }
            }

            ExecutorCommand::CancelOrders(order_ids) => {
                result.merge(self.execute_cancellations(&order_ids));
            }

            ExecutorCommand::CancelAllForToken(token_id) => {
                match self.runtime.block_on(self.trading.cancel_market_orders(None, Some(&token_id))) {
                    Ok(r) => {
                        result.cancelled_count = r.canceled.len();
                        result.cancelled_ids = r.canceled;
                    }
                    Err(e) => result.add_error("cancel_token", e.to_string()),
                }
            }

            ExecutorCommand::CancelAll => {
                match self.runtime.block_on(self.trading.cancel_all()) {
                    Ok(r) => {
                        result.cancelled_count = r.canceled.len();
                        result.cancelled_ids = r.canceled;
                    }
                    Err(e) => result.add_error("cancel_all", e.to_string()),
                }
            }

            ExecutorCommand::PlaceLimit(order) => {
                result.merge(self.execute_limits(&[order]));
            }

            ExecutorCommand::Merge { condition_id, amount } => {
                if amount <= 0.0 {
                    result.add_error("merge", format!("Invalid merge amount: {}", amount));
                    error!("[Executor] Invalid merge amount: {}", amount);
                } else {
                    let raw_amount = usdc_to_raw(amount);
                    match self.runtime.block_on(ctf_merge(&condition_id, false, raw_amount)) {
                        Ok(tx_hash) => {
                            result.merge_tx = Some(format!("{:x}", tx_hash));
                            info!("[Executor] Merge tx: {:x}", tx_hash);
                        }
                        Err(e) => {
                            result.add_error("merge", e.to_string());
                            error!("[Executor] Merge failed: {}", e);
                        }
                    }
                }
            }

            ExecutorCommand::Shutdown => {
                // Handled in run loop
            }
        }

        result
    }

    /// Execute batch cancellations
    fn execute_cancellations(&self, order_ids: &[String]) -> ExecutorResult {
        let mut result = ExecutorResult::new();
        if order_ids.is_empty() {
            return result;
        }

        debug!("[Executor] Cancelling {} orders", order_ids.len());

        match self.runtime.block_on(self.trading.cancel_orders(order_ids)) {
            Ok(response) => {
                result.cancelled_count = response.canceled.len();
                result.cancelled_ids = response.canceled;

                // Process not_canceled: distinguish between "order gone" (success) vs real errors
                // "Order gone" means the order is already off the book - that's what we wanted!
                if !response.not_canceled.is_empty() {
                    let mut real_errors = 0;
                    for (id, reason) in response.not_canceled {
                        let reason_lower = reason.to_lowercase();

                        // These are SUCCESS conditions - order is gone from the book
                        if reason_lower.contains("matched")
                            || reason_lower.contains("already canceled")
                            || reason_lower.contains("can't be found")
                            || reason_lower.contains("not found")
                        {
                            // Order is effectively cancelled (no longer on book)
                            debug!(
                                "[Executor] Order {} already gone: {}",
                                &id[..16.min(id.len())], reason
                            );
                            // Count as effectively cancelled
                            result.cancelled_count += 1;
                            result.cancelled_ids.push(id);
                        } else {
                            // Real error - log and track
                            real_errors += 1;
                            result.add_error("cancel", format!("{}: {}", id, reason));
                            warn!(
                                "[Executor] Cancel error for {}: {}",
                                &id[..16.min(id.len())], reason
                            );
                        }
                    }
                    if real_errors > 0 {
                        warn!("[Executor] {} cancel requests had real errors", real_errors);
                    }
                }

                if result.cancelled_count > 0 {
                    info!("[Executor] Cancelled {} orders", result.cancelled_count);

                    // FIX: Update OMS directly after REST confirms cancellation.
                    // This fixes the critical issue where WebSocket CANCELLATION messages
                    // are delayed or dropped, causing the OMS to keep stale "Open" status.
                    if let Some(ref order_state) = self.order_state {
                        let updated = order_state.write().mark_orders_cancelled(&result.cancelled_ids);
                        if updated > 0 {
                            debug!("[Executor] Updated OMS: marked {} orders as cancelled", updated);
                        }
                    }
                }
            }
            Err(e) => {
                result.add_error("cancel_orders", e.to_string());
                error!("[Executor] Cancel failed: {}", e);
            }
        }

        result
    }

    /// Execute limit orders individually (more reliable than batch)
    fn execute_limits(&self, orders: &[LimitOrder]) -> ExecutorResult {
        let mut result = ExecutorResult::new();
        if orders.is_empty() {
            return result;
        }

        debug!("[Executor] Placing {} limit orders individually", orders.len());

        // Place each order individually for reliability
        for order in orders {
            let token_short = &order.token_id[..8.min(order.token_id.len())];

            info!(
                "[Executor] Placing: {} @ ${:.2} for {:.1} shares",
                token_short, order.price, order.size
            );

            let place_result = match order.side {
                Side::Buy => self.runtime.block_on(
                    self.trading.buy(&order.token_id, order.price, order.size)
                ),
                Side::Sell => self.runtime.block_on(
                    self.trading.sell(&order.token_id, order.price, order.size)
                ),
            };

            match place_result {
                Ok(response) => {
                    if response.success {
                        result.placed_count += 1;
                        if let Some(ref order_id) = response.order_id {
                            result.placed_ids.push(order_id.clone());
                        }
                        info!(
                            "[Executor] ✓ Placed {} @ ${:.2} → {:?}",
                            token_short, order.price, response.status
                        );
                    } else {
                        let err_msg = response.error_msg.unwrap_or_else(|| "Unknown error".to_string());
                        result.add_error("place_limit", format!("{}: {}", token_short, err_msg));
                        warn!(
                            "[Executor] ✗ Failed {} @ ${:.2}: {}",
                            token_short, order.price, err_msg
                        );
                    }
                }
                Err(e) => {
                    result.add_error("place_limit", format!("{}: {}", token_short, e));
                    error!(
                        "[Executor] ✗ Error placing {} @ ${:.2}: {}",
                        token_short, order.price, e
                    );
                }
            }
        }

        info!("[Executor] Placed {}/{} orders", result.placed_count, orders.len());
        result
    }
}

/// Errors from executor operations
#[derive(Debug, Clone)]
pub enum ExecutorError {
    /// Command channel is closed
    ChannelClosed,
    /// Executor thread panicked
    ThreadPanic,
    /// Shutdown timed out
    ShutdownTimeout,
    /// Trading client error
    TradingError(String),
}

impl std::fmt::Display for ExecutorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecutorError::ChannelClosed => write!(f, "Executor channel closed"),
            ExecutorError::ThreadPanic => write!(f, "Executor thread panicked"),
            ExecutorError::ShutdownTimeout => write!(f, "Executor shutdown timed out"),
            ExecutorError::TradingError(e) => write!(f, "Trading error: {}", e),
        }
    }
}

impl std::error::Error for ExecutorError {}

// Tests require TradingClient - run as integration tests
// #[cfg(test)]
// mod tests {
//     use super::*;
//
//     #[test]
//     #[ignore] // Requires TradingClient
//     fn test_executor_spawn_and_shutdown() {
//         // Need to provide Arc<TradingClient> to spawn()
//     }
// }
