//! Solver - pure function that takes inputs and returns actions.

mod core;
mod quotes;
mod diff;
mod taker;
mod profitability;

pub use core::solve;
pub use quotes::calculate_quotes;
pub use diff::diff_orders;
pub use taker::find_taker_opportunity;
pub use profitability::validate_profitability;
