//! Inventory MM Strategy
//!
//! Inventory-balanced market making for Up/Down binary markets.

mod config;
mod strategy;
pub mod components;
pub mod types;
pub mod quoter;

// Re-exports for convenience
pub use config::{InventoryMMConfig, MarketSpec};
pub use strategy::{InventoryMMStrategy, extract_solver_input};
pub use components::{solve, Executor, ExecutorHandle, QuoterExecutorHandle, Merger, MergerConfig, MergeDecision};
pub use types::{
    SolverInput, SolverOutput, SolverConfig,
    InventorySnapshot, OrderbookSnapshot, OrderSnapshot, OpenOrder,
    Quote, LimitOrder, TakerOrder, QuoteLadder, Side,
};
pub use quoter::{Quoter, QuoterContext, MarketInfo, QuoterWsConfig, QuoterWsClient, build_quoter_ws_client};
