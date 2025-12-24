//! Strategy trait definition
//!
//! Defines the contract that all sniper strategies must implement.

use crate::infrastructure::BalanceManager;
use crate::infrastructure::client::clob::TradingClient;
use crate::infrastructure::client::user::{SharedOrderState, SharedPositionTracker};
use crate::infrastructure::database::{DatabaseError, MarketDatabase};
use crate::infrastructure::shutdown::ShutdownManager;
use async_trait::async_trait;
use parking_lot::RwLock;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use thiserror::Error;

/// Result type for strategy operations
pub type StrategyResult<T> = Result<T, StrategyError>;

/// Errors that can occur in strategy execution
#[derive(Debug, Error)]
pub enum StrategyError {
    #[error("Database error: {0}")]
    Database(#[from] DatabaseError),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Strategy interrupted by shutdown")]
    Shutdown,

    #[error("Strategy error: {0}")]
    Other(#[from] anyhow::Error),
}

/// Context provided to all strategies
pub struct StrategyContext {
    /// Database access
    pub database: Arc<MarketDatabase>,
    /// Shutdown flag for graceful termination
    pub shutdown_flag: Arc<AtomicBool>,
    /// Shutdown manager for interruptible operations
    pub shutdown: Arc<ShutdownManager>,
    /// Trading client for order placement
    pub trading: Arc<TradingClient>,
    /// Balance manager for monitoring and halt control
    pub balance_manager: Arc<RwLock<BalanceManager>>,
    /// Real-time order state from user WebSocket
    pub order_state: SharedOrderState,
    /// Real-time position tracker
    pub position_tracker: SharedPositionTracker,
}

impl StrategyContext {
    pub fn new(
        database: Arc<MarketDatabase>,
        shutdown: Arc<ShutdownManager>,
        trading: Arc<TradingClient>,
        balance_manager: Arc<RwLock<BalanceManager>>,
        order_state: SharedOrderState,
        position_tracker: SharedPositionTracker,
    ) -> Self {
        Self {
            database,
            shutdown_flag: shutdown.flag(),
            shutdown,
            trading,
            balance_manager,
            order_state,
            position_tracker,
        }
    }

    /// Check if the strategy should continue running
    pub fn is_running(&self) -> bool {
        self.shutdown.is_running()
    }

    /// Check if trading is halted due to balance drop
    pub fn is_trading_halted(&self) -> bool {
        self.balance_manager.read().is_halted()
    }
}

/// Trait that all sniper strategies must implement
#[async_trait]
pub trait Strategy: Send + Sync {
    /// Get the strategy name for logging and identification
    fn name(&self) -> &str;

    /// Get a description of what this strategy does
    fn description(&self) -> &str;

    /// Start the strategy execution
    ///
    /// This method should run the main strategy loop until:
    /// - The shutdown flag is set to false
    /// - An unrecoverable error occurs
    ///
    /// The strategy should check `ctx.is_running()` regularly and
    /// use `ctx.shutdown.interruptible_sleep()` for delays.
    async fn start(&mut self, ctx: &StrategyContext) -> StrategyResult<()>;

    /// Stop the strategy gracefully
    ///
    /// Called when shutdown is requested. The strategy should:
    /// - Stop accepting new work
    /// - Complete or abort in-flight operations
    /// - Clean up any resources
    ///
    /// The default implementation does nothing (relies on shutdown flag).
    async fn stop(&mut self) -> StrategyResult<()> {
        Ok(())
    }

    /// Optional: Called once before `start()` for initialization
    async fn initialize(&mut self, _ctx: &StrategyContext) -> StrategyResult<()> {
        Ok(())
    }
}
