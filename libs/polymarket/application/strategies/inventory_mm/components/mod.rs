//! Components for the Inventory MM strategy.

pub mod solver;
pub mod executor;
pub mod merger;

pub use solver::solve;
pub use executor::{Executor, ExecutorHandle};
pub use merger::{Merger, MergerConfig, MergeDecision};
