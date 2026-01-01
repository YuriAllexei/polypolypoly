//! Solver - pure function that takes inputs and returns actions.

mod core;
mod quotes;
mod diff;
mod profitability;

#[cfg(test)]
mod test_solver_visual;

pub use core::solve;
pub use quotes::calculate_quotes;
pub use diff::diff_orders;
pub use profitability::{calculate_max_bids, is_inventory_unprofitable, check_recovery_status, RecoveryStatus};
