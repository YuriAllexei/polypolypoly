//! Executor - runs on its own thread, processes commands via channel.

use std::thread::{self, JoinHandle};
use crossbeam_channel::{unbounded, Sender, Receiver};
use tracing::{info, warn, error, debug};

use super::commands::{ExecutorCommand, ExecutorResult};
use crate::application::strategies::inventory_mm::types::{SolverOutput, LimitOrder, TakerOrder};

/// Handle to communicate with the Executor thread
pub struct ExecutorHandle {
    /// Channel to send commands to executor
    command_tx: Sender<ExecutorCommand>,
    /// Thread handle for joining on shutdown
    thread_handle: Option<JoinHandle<()>>,
}

impl ExecutorHandle {
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
        !self.command_tx.is_empty() || self.thread_handle.as_ref().map(|h| !h.is_finished()).unwrap_or(false)
    }
}

/// The Executor that runs on its own thread
pub struct Executor {
    /// Receiver for commands
    command_rx: Receiver<ExecutorCommand>,
    // TODO: Add TradingClient when integrating
    // trading_client: Arc<TradingClient>,
}

impl Executor {
    /// Spawn the executor on a new thread
    ///
    /// Returns a handle for communication
    pub fn spawn() -> ExecutorHandle {
        let (command_tx, command_rx) = unbounded();

        let executor = Executor { command_rx };

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

    /// Spawn with a trading client for actual execution
    /// TODO: Uncomment when integrating with TradingClient
    // pub fn spawn_with_client(trading_client: Arc<TradingClient>) -> ExecutorHandle {
    //     let (command_tx, command_rx) = unbounded();
    //     let executor = Executor { command_rx, trading_client };
    //     // ... spawn thread
    // }

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
                            "[Executor] Completed: cancelled={}, placed={}, takers={}",
                            result.cancelled_count, result.placed_count, result.taker_count
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
                // Process cancellations first (safer order per TBD discussion)
                if !output.cancellations.is_empty() {
                    let cancel_result = self.execute_cancellations(&output.cancellations);
                    result.merge(cancel_result);
                }

                // Then taker orders (time-sensitive)
                for taker in output.taker_orders {
                    let taker_result = self.execute_taker(&taker);
                    result.merge(taker_result);
                }

                // Finally limit orders
                for limit in output.limit_orders {
                    let limit_result = self.execute_limit(&limit);
                    result.merge(limit_result);
                }
            }

            ExecutorCommand::CancelOrders(order_ids) => {
                result = self.execute_cancellations(&order_ids);
            }

            ExecutorCommand::CancelAllForToken(token_id) => {
                // TODO: Implement when TradingClient available
                warn!("[Executor] CancelAllForToken not yet implemented: {}", token_id);
            }

            ExecutorCommand::CancelAll => {
                // TODO: Implement when TradingClient available
                warn!("[Executor] CancelAll not yet implemented");
            }

            ExecutorCommand::PlaceLimit(order) => {
                result = self.execute_limit(&order);
            }

            ExecutorCommand::ExecuteTaker(order) => {
                result = self.execute_taker(&order);
            }

            ExecutorCommand::Shutdown => {
                // Handled in run loop
            }
        }

        result
    }

    /// Execute cancellations
    fn execute_cancellations(&self, order_ids: &[String]) -> ExecutorResult {
        let mut result = ExecutorResult::new();

        // TODO: Replace with actual TradingClient call
        // let response = self.trading_client.cancel_orders(order_ids).await;

        for order_id in order_ids {
            debug!("[Executor] Would cancel order: {}", order_id);
            // Simulate success for now
            result.cancelled_count += 1;
            result.cancelled_ids.push(order_id.clone());
        }

        result
    }

    /// Execute a limit order
    fn execute_limit(&self, order: &LimitOrder) -> ExecutorResult {
        let mut result = ExecutorResult::new();

        // TODO: Replace with actual TradingClient call
        // let response = self.trading_client.buy(&order.token_id, order.price, order.size).await;

        debug!(
            "[Executor] Would place limit: {} {} @ {} size {}",
            order.side, order.token_id, order.price, order.size
        );

        // Simulate success for now
        result.placed_count += 1;
        result.placed_ids.push(format!("simulated_{}", order.price));

        result
    }

    /// Execute a taker order (FOK)
    fn execute_taker(&self, order: &TakerOrder) -> ExecutorResult {
        let mut result = ExecutorResult::new();

        // TODO: Replace with actual TradingClient call
        // let response = self.trading_client.buy_fok(&order.token_id, order.price, order.size).await;

        debug!(
            "[Executor] Would execute taker: {} {} @ {} size {} (score: {:.2})",
            order.side, order.token_id, order.price, order.size, order.score
        );

        // Simulate success for now
        result.taker_count += 1;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::strategies::inventory_mm::types::Side;

    #[test]
    fn test_executor_spawn_and_shutdown() {
        let handle = Executor::spawn();

        // Send a simple command
        handle.send(ExecutorCommand::CancelOrders(vec!["test".to_string()])).unwrap();

        // Shutdown
        handle.shutdown().unwrap();
    }

    #[test]
    fn test_executor_execute_batch() {
        let handle = Executor::spawn();

        let output = SolverOutput {
            cancellations: vec!["order1".to_string()],
            limit_orders: vec![
                LimitOrder::buy("token".to_string(), 0.54, 100.0),
            ],
            taker_orders: vec![],
        };

        handle.execute(output).unwrap();

        // Give it time to process
        std::thread::sleep(std::time::Duration::from_millis(10));

        handle.shutdown().unwrap();
    }

    #[test]
    fn test_executor_empty_output() {
        let handle = Executor::spawn();

        // Empty output should not send anything
        let output = SolverOutput::default();
        handle.execute(output).unwrap();

        handle.shutdown().unwrap();
    }
}
