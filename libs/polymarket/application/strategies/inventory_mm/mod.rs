//! Inventory MM Strategy
//!
//! Inventory-balanced market making for Up/Down binary markets.

mod config;
mod strategy;
pub mod components;
pub mod types;

// Re-exports for convenience
pub use config::InventoryMMConfig;
pub use strategy::{InventoryMMStrategy, extract_solver_input};
pub use components::{solve, Executor, ExecutorHandle, Merger, MergerConfig, MergeDecision};
pub use types::{
    SolverInput, SolverOutput, SolverConfig,
    InventorySnapshot, OrderbookSnapshot, OrderSnapshot, OpenOrder,
    Quote, LimitOrder, TakerOrder, QuoteLadder, Side,
};
