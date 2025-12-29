//! Executor - runs on its own thread, processes commands via channel.

use std::sync::Arc;
use std::thread::{self, JoinHandle};
use crossbeam_channel::{unbounded, Sender, Receiver};
use tokio::runtime::Runtime;
use tracing::{info, error, debug};

use super::commands::{ExecutorCommand, ExecutorResult};
use crate::application::strategies::inventory_mm::types::{SolverOutput, LimitOrder, Side};
use crate::infrastructure::client::clob::{TradingClient, Side as TradingSide, OrderType};
use crate::infrastructure::client::ctf::{merge as ctf_merge, usdc_to_raw};

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

    /// Shutdown the executor gracefully
    pub fn shutdown(mut self) -> Result<(), ExecutorError> {
        // Send shutdown command
        let _ = self.send(ExecutorCommand::Shutdown);

        // Wait for thread to finish
        if let Some(handle) = self.thread_handle.take() {
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
}

impl Executor {
    /// Spawn the executor on a new thread with a trading client
    pub fn spawn(trading: Arc<TradingClient>) -> ExecutorHandle {
        let (command_tx, command_rx) = unbounded();

        let runtime = Runtime::new().expect("Failed to create tokio runtime");

        let executor = Executor {
            command_rx,
            trading,
            runtime,
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
                if !response.not_canceled.is_empty() {
                    for (id, reason) in response.not_canceled {
                        result.add_error("cancel", format!("{}: {}", id, reason));
                    }
                }
                info!("[Executor] Cancelled {} orders", result.cancelled_count);
            }
            Err(e) => {
                result.add_error("cancel_orders", e.to_string());
                error!("[Executor] Cancel failed: {}", e);
            }
        }

        result
    }

    /// Execute batch limit orders
    fn execute_limits(&self, orders: &[LimitOrder]) -> ExecutorResult {
        let mut result = ExecutorResult::new();
        if orders.is_empty() {
            return result;
        }

        debug!("[Executor] Placing {} limit orders", orders.len());

        // Convert to TradingClient format
        let batch: Vec<_> = orders.iter()
            .map(|o| (
                o.token_id.clone(),
                o.price,
                o.size,
                match o.side {
                    Side::Buy => TradingSide::Buy,
                    Side::Sell => TradingSide::Sell,
                },
                OrderType::GTC,
            ))
            .collect();

        match self.runtime.block_on(self.trading.place_batch_orders(batch, None)) {
            Ok(batch_result) => {
                result.placed_count = batch_result.success_count();
                result.placed_ids = batch_result.order_ids();
                for (token_id, msg) in batch_result.error_messages() {
                    result.add_error("place_limit", format!("{}: {}", token_id, msg));
                }
                info!("[Executor] Placed {} limit orders", result.placed_count);
            }
            Err(e) => {
                result.add_error("place_batch", e.to_string());
                error!("[Executor] Batch placement failed: {}", e);
            }
        }

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
    /// Trading client error
    TradingError(String),
}

impl std::fmt::Display for ExecutorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecutorError::ChannelClosed => write!(f, "Executor channel closed"),
            ExecutorError::ThreadPanic => write!(f, "Executor thread panicked"),
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
